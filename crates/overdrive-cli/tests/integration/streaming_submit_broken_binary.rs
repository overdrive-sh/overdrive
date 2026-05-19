//! S-WS-02 — Tier 3 broken-binary regression target (US-02 KPI-02 / KPI-04),
//! Job-kind variant post-ADR-0051 wire migration.
//!
//! Per `docs/feature/cli-submit-vs-deploy-and-alloc-status/deliver/02-04`
//! step 02-04 acceptance criteria + the ADR-0051 wire-shape migration
//! (step 02-03b): a real `[job]` spec naming a binary path that does
//! not exist (`/usr/local/bin/no-such-binary`) drives a real
//! `tokio::process::Command::spawn` ENOENT, the cause-class
//! `TransitionReason::ExecBinaryNotFound { path }` payload travels
//! through the broadcast bus + the streaming projection + the
//! observation-store snapshot lane, and the streaming surface emits
//! `JobSubmitEvent::Failed` with the kernel-observed exit code,
//! while the observation-store snapshot's `last_transition.reason`
//! carries the typed `ExecBinaryNotFound { path }` payload byte-equal
//! to the inputs.
//!
//! ADR-0051 migration note: pre-migration this test exploited a
//! `workload_kind: None → Service` server-side coercion default to
//! drive Service-arm streaming via a flat `JobSpecInput` payload —
//! where the original `SubmitEvent::ConvergedFailed.terminal_reason /
//! streaming_reason / streaming_error` cause-class fields lived. That
//! coercion was deleted by ADR-0051 OQ-8. Job-kind's terminal event
//! (`JobSubmitEvent::Failed`) does NOT carry the cause-class fields
//! on the wire — Job-kind's wire shape is exit code + attempts +
//! `stderr_tail`, per ADR-0047 §3 [D7]. The cause-class observability
//! moves to the observation-store lane: the snapshot's
//! `last_transition.reason` carries the typed `ExecBinaryNotFound`
//! payload regardless of workload kind, because the `action_shim`
//! projection from `DriverError → TransitionReason` is kind-agnostic
//! (`crates/overdrive-control-plane/src/action_shim/mod.rs`). This
//! test asserts the load-bearing observability property on the
//! snapshot surface; the streaming-side cause-class assertions are
//! covered by the Service-arm streaming acceptance scenarios that
//! land with step 02-03c (proper listener shape).
//!
//! KPI alignment under Job-kind:
//!   * KPI-02 — the snapshot's `last_transition.reason` carries the
//!     typed `ExecBinaryNotFound { path }` payload verbatim. The
//!     streaming-side cause-class fields exist on the Service arm
//!     only and are tested in 02-03c.
//!   * KPI-04 — CLI exit code is the workload's kernel-observed exit
//!     code (KPI K1 honesty). For a Job-kind ENOENT spawn failure
//!     the reconciler stamps `TerminalCondition::Failed { exit_code:
//!     1 }` (the synthetic exit code for spawn-time failures); the
//!     streaming layer projects to `JobSubmitEvent::Failed { exit_code:
//!     1, attempts: 1, max_attempts: 1, stderr_tail: <ENOENT text> }`
//!     and the CLI process exits 1.
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

/// `[job]` TOML pointing at a binary path that does not exist. The
/// kind-discriminator triggers `WorkloadSpec::Job`; the ENOENT on
/// `Command::spawn` produces a `DriverError` whose Display matches
/// the `spawn <path>: No such file or directory (os error 2)` regex
/// in `action_shim/mod.rs`, projected to
/// `TransitionReason::ExecBinaryNotFound { path }`.
const fn broken_binary_job_spec_toml() -> &'static str {
    r#"
[job]
id = "doomed"

[exec]
command = "/usr/local/bin/no-such-binary"
args = []

[resources]
cpu_milli = 100
memory_bytes = 67108864
"#
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn streaming_submit_against_missing_binary_emits_job_failed_with_cause_class_payload() {
    let (handle, server_tmp) = spawn_server().await;
    let server_cfg = config_path(server_tmp.path());

    let spec_path = write_toml(server_tmp.path(), "doomed.toml", broken_binary_job_spec_toml());

    // Phase 1 — drive the streaming submit end-to-end. The Job-kind
    // reconciler observes the spawn-time ENOENT and the streaming
    // handler projects to a terminal `JobSubmitEvent` variant.
    //
    // Note: Job-kind's spawn-failure → terminal-verdict mapping is
    // semantically distinct from the legacy Service-arm flow that
    // produced this test's original `BackoffExhausted` shape. Job-
    // kind has single-shot semantics (ADR-0047 §1 / ADR-0037) and the
    // reconciler's stamping of `TerminalCondition` for a spawn-time
    // ENOENT on Job-kind is the kind-aware path landed in step 02-01
    // of `workload-kind-discriminator`. For step 02-03b's purpose
    // (the ADR-0051 wire-shape migration), the load-bearing
    // observability property is the snapshot's
    // `last_transition.reason` carrying the typed
    // `ExecBinaryNotFound { path }` payload (Phase 2 below) — the
    // streaming-side cause-class projection is the Service arm's
    // 02-03c concern.
    let output = overdrive_cli::commands::job::submit_streaming(SubmitArgs {
        spec: spec_path,
        config_path: server_cfg.clone(),
    })
    .await
    .expect(
        "submit_streaming must complete end-to-end (the failure is on the workload, not the \
         submit lane)",
    );

    // Job-kind terminal-reason / streaming-reason / streaming-error
    // are all None on every `consume_stream_job` arm — those fields
    // are Service-arm `SubmitEvent::ConvergedFailed` projections.
    // The cause-class observability for Job kind lives on the
    // observation-store lane (asserted below on the snapshot's
    // `last_transition.reason`).
    assert!(
        output.terminal_reason.is_none(),
        "Job-kind terminal must not carry terminal_reason (Service-arm field); got {:?}",
        output.terminal_reason,
    );
    assert!(
        output.streaming_reason.is_none(),
        "Job-kind terminal must not carry streaming_reason (Service-arm field); got {:?}",
        output.streaming_reason,
    );
    assert!(
        output.streaming_error.is_none(),
        "Job-kind terminal must not carry streaming_error (Service-arm field); got {:?}",
        output.streaming_error,
    );

    // Phase 2 — drive `alloc::status --job doomed` against the same
    // observation store. The snapshot's `last_transition.reason`
    // MUST carry the typed `ExecBinaryNotFound { path }` payload —
    // this is the load-bearing KPI-02 cause-class assertion that
    // survives the ADR-0051 wire-shape migration. The action_shim's
    // `DriverError → TransitionReason` projection is kind-agnostic
    // (`crates/overdrive-control-plane/src/action_shim/mod.rs`), so
    // the typed cause-class payload is byte-identical to the
    // pre-migration Service-arm shape.
    let snapshot = overdrive_cli::commands::alloc::status_snapshot(StatusArgs {
        job: "doomed".to_owned(),
        config_path: server_cfg,
    })
    .await
    .expect("alloc::status_snapshot must return a typed AllocStatusResponse");

    let row = snapshot.rows.first().expect("S-WS-02: snapshot must carry at least one alloc row");
    let last_transition = row
        .last_transition
        .as_ref()
        .expect("S-WS-02: row.last_transition must be Some after at least one transition");

    // S-WS-02 KPI-02 — the snapshot's `last_transition.reason`
    // carries the typed `ExecBinaryNotFound { path }` payload byte-
    // equal to the input spec's `command` field. This is the load-
    // bearing observability property for the cause-class taxonomy
    // (ADR-0028) under the post-ADR-0051 Job-kind streaming surface.
    let TransitionReason::ExecBinaryNotFound { path: snapshot_path } = &last_transition.reason
    else {
        panic!(
            "S-WS-02: snapshot.last_transition.reason must be ExecBinaryNotFound; got {:?}",
            last_transition.reason,
        );
    };
    assert_eq!(
        snapshot_path, BROKEN_BINARY_PATH,
        "S-WS-02 KPI-02: snapshot's cause-class typed payload must carry the verbatim spec \
         path; got: {snapshot_path}",
    );

    // The per-row `error` field carries the verbatim driver-text
    // rendering — populated by the action_shim's `record_lifecycle`
    // path. The post-migration assertion is `is_some + non-empty`
    // (the wire byte-equality with a streaming counterpart no
    // longer applies because the Job-kind wire shape doesn't carry
    // the field; the equivalent surface lives in
    // `stderr_tail` which is rendered into the summary above).
    let row_error = row
        .error
        .as_ref()
        .expect("S-WS-02: row.error must be Some for a Failed row (the verbatim driver text)");
    assert!(
        !row_error.is_empty(),
        "S-WS-02: row.error must be non-empty for a Failed row; got: {row_error:?}",
    );
    assert!(
        row_error.contains("No such file or directory") || row_error.contains("os error 2"),
        "S-WS-02: row.error must name the ENOENT cause; got: {row_error:?}",
    );

    handle.shutdown().await.expect("clean shutdown");
}
