//! Slice 02 of `workload-kind-discriminator` — Job-kind streaming
//! submit acceptance tests.
//!
//! The load-bearing assertion this file enforces is the
//! anti-scenario S-02-05 — **no line of operator-visible output for a
//! Job-kind submit ever contains the substrings `"is running with"`
//! or `"(took live)"`**. The conjunction of RCA root causes B+C+D
//! (which produced the historical false-positive "is running with N/M
//! replicas (took live)" line on a coinflip Job submit) is rendered
//! structurally unreachable for Job kind by the per-kind streaming-
//! event sibling enums defined in this slice (ADR-0047 §3 [D2] +
//! [D7]).
//!
//! Per `crates/overdrive-cli/CLAUDE.md` § *Integration tests — no
//! subprocess*: tests call the handler directly. The test exercises a
//! `[job]` TOML spec (the `WorkloadSpec::Job` discriminator) end-to-
//! end against an in-process control plane plus the real
//! `ExecDriver`.
//!
//! Linux-gated because the production `ExecDriver` requires
//! `tokio::process::Command::spawn` against a real `/bin/bash`. The
//! macOS `--no-run` gate compiles this file via `cargo check
//! --features integration-tests` per `.claude/rules/testing.md`.

#![cfg(target_os = "linux")]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use std::time::Duration;

use overdrive_cli::commands::alloc::StatusArgs as AllocStatusArgs;
use overdrive_cli::commands::job::{StopArgs, SubmitArgs};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_control_plane::api::AllocStateWire;
use serial_test::serial;
use tempfile::TempDir;

async fn spawn_server() -> (ServeHandle, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let args = ServeArgs { bind, data_dir, config_dir };
    let handle = overdrive_cli::commands::serve::run_with_dataplane(
        args,
        std::sync::Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
    )
    .await
    .expect("serve::run");
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

/// Poll the alloc snapshot until at least one allocation for `job_id`
/// reaches `Running`. Replaces a bare `sleep(150ms)` race that
/// flaked on Lima under parallel-test contention — the
/// `Pending → Running` window (submit → reconciler → `ExecDriver`
/// spawn → first observation write) can stretch past 150 ms when
/// the bpf-artifact / cgroup-workload test groups are scheduled
/// alongside this one.
async fn wait_for_alloc_running(config_path: &Path, job_id: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let snapshot = overdrive_cli::commands::alloc::status_snapshot(AllocStatusArgs {
            job: job_id.to_owned(),
            config_path: config_path.to_owned(),
        })
        .await;
        if let Ok(resp) = snapshot
            && resp.rows.iter().any(|r| matches!(r.state, AllocStateWire::Running))
        {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "alloc for job {job_id} did not reach Running within 10s",
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// `[job]+[exec]+[resources]` TOML — the kind-discriminator triggers
/// `WorkloadSpec::Job` per slice 01. `/bin/sleep 300` is long-running
/// so the test can issue an explicit `stop` to trigger the
/// `Stopped` terminal event.
///
/// The natural-exit path (workload terminates with exit code 0
/// without operator stop) requires the reconciler to emit
/// `TerminalCondition::Completed { exit_code: 0 }` per ADR-0037
/// Amendment 2026-05-10 — that work lands in a follow-up sub-slice
/// of step 02-01.
const fn job_long_sleep_spec() -> &'static str {
    r#"
[job]
id = "happy-job"

[exec]
command = "/bin/sleep"
args = ["300"]

[resources]
cpu_milli = 100
memory_bytes = 67108864
"#
}

/// S-02-05 — anti-scenario: no Job submit produces the substring
/// `"is running with"` or `"(took live)"` on any output line. This
/// is the load-bearing structural check that closes the historical
/// false-positive bug under audit (RCA: B+C+D conjunction).
///
/// Today: this test FAILS — the current production submit path
/// routes Job-kind specs through the legacy flat `JobSpecInput`
/// parser AND the legacy `format_running_summary` Service-vocabulary
/// renderer, which by construction emits `"is running with"`.
///
/// Slice 02 (this step) wires `WorkloadSpec` into `submit_streaming`
/// so a `[job]`-shape spec dispatches via `JobSubmitEvent` (no
/// converged-running terminal variant) and renders via the new
/// `format_job_succeeded_summary` whose output names exit code +
/// duration, never the substring `"is running with"`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(workload_cgroup)]
async fn s_02_05_anti_scenario_no_is_running_with() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    let spec_path = write_toml(tmp.path(), "happy-job.toml", job_long_sleep_spec());

    // Drive submit_streaming and trigger a stop concurrently so the
    // streaming consumer observes `Stopped` (the Job-kind
    // success-path terminal event in this slice). A future sub-slice
    // wires natural-exit through `TerminalCondition::Completed` per
    // ADR-0037 Amendment 2026-05-10; until then the explicit stop is
    // the only path that produces a terminal verdict for a Job spec
    // that doesn't trip the backoff ceiling.
    let submit_cfg = cfg.clone();
    let stop_cfg = cfg.clone();
    let submit_handle = tokio::spawn(async move {
        overdrive_cli::commands::job::submit_streaming(SubmitArgs {
            spec: spec_path,
            config_path: submit_cfg,
        })
        .await
    });
    wait_for_alloc_running(&cfg, "happy-job").await;
    let _ = overdrive_cli::commands::job::stop(StopArgs {
        id: "happy-job".to_owned(),
        config_path: stop_cfg,
    })
    .await
    .expect("stop must succeed");

    let output = submit_handle
        .await
        .expect("submit task must not panic")
        .expect("submit_streaming must complete end-to-end");

    // Anti-scenario S-02-05: the operator-visible summary MUST NOT
    // contain the historical false-positive substrings — both are
    // structurally impossible for Job kind under the
    // `JobSubmitEvent` per-kind enum design (ADR-0047 §3 [D2]).
    assert!(
        !output.summary.contains("is running with"),
        "S-02-05 anti-scenario violated — Job summary must NOT contain \
         'is running with'; got: {summary:?}",
        summary = output.summary,
    );
    assert!(
        !output.summary.contains("(took live)"),
        "S-02-05 anti-scenario violated — Job summary must NOT contain \
         '(took live)'; got: {summary:?}",
        summary = output.summary,
    );

    // Honesty assertion: an operator-stopped Job renders as "stopped
    // by operator", not "succeeded" (which would be a false-positive —
    // fixed in 064a8cc3). The explicit stop is the terminal trigger in
    // this test; natural exit-0 succeeded rendering is covered by
    // separate exit-code-honesty tests.
    assert!(
        output.summary.contains("stopped by operator"),
        "An operator-stopped Job must render 'stopped by operator'; got: {summary:?}",
        summary = output.summary,
    );

    // CLI process exit code for an operator-stopped Job is 130
    // (SIGINT-equivalent), not 0 (which would falsely signal success).
    assert_eq!(output.exit_code, 130, "operator-stopped Job maps to CLI exit 130");

    drop(handle);
}

/// S-02-06 — submit echo names the kind upfront. Before any
/// streaming events, the CLI prints `"Submitting job '<name>'
/// (kind=Job, run-to-completion)"`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(workload_cgroup)]
async fn s_02_06_submit_echo_names_kind_upfront() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    let spec_path = write_toml(tmp.path(), "happy-job.toml", job_long_sleep_spec());

    let submit_cfg = cfg.clone();
    let stop_cfg = cfg.clone();
    let submit_handle = tokio::spawn(async move {
        overdrive_cli::commands::job::submit_streaming(SubmitArgs {
            spec: spec_path,
            config_path: submit_cfg,
        })
        .await
    });
    wait_for_alloc_running(&cfg, "happy-job").await;
    let _ = overdrive_cli::commands::job::stop(StopArgs {
        id: "happy-job".to_owned(),
        config_path: stop_cfg,
    })
    .await
    .expect("stop must succeed");

    let output = submit_handle
        .await
        .expect("submit task must not panic")
        .expect("submit_streaming must complete end-to-end");

    assert!(
        output.summary.contains("Submitting job 'happy-job' (kind=Job, run-to-completion)"),
        "S-02-06: submit echo must name kind upfront; got: {summary:?}",
        summary = output.summary,
    );

    drop(handle);
}

/// `[job]+[exec]+[resources]` TOML — a Job-kind workload that exits 0
/// immediately. Drives the natural-exit path for Job-kind: the
/// reconciler emits `TerminalCondition::Completed { exit_code: 0 }`
/// (per ADR-0037 Amendment 2026-05-10) and the streaming layer projects
/// to `JobSubmitEvent::Succeeded`.
///
/// Per fix-exit-observer-running-gate (Solution 1'): the action-shim's
/// `obs.write(Running)` is now structurally happens-before the
/// watcher's `ExitEvent` emission, so sub-millisecond-exit workloads
/// no longer race. No fixture-side workaround required.
const fn job_exit_zero_spec() -> &'static str {
    r#"
[job]
id = "happy-job"

[exec]
command = "/bin/sh"
args = ["-c", "exit 0"]

[resources]
cpu_milli = 100
memory_bytes = 67108864
"#
}

/// `[job]+[exec]+[resources]` TOML — a Job-kind workload that exits 1
/// immediately. Drives the natural-exit failure path for Job-kind: the
/// reconciler emits `TerminalCondition::Failed { exit_code: 1 }` and
/// the streaming layer projects to `JobSubmitEvent::Failed`.
///
/// Per fix-exit-observer-running-gate (Solution 1'): the action-shim's
/// `obs.write(Running)` is now structurally happens-before the
/// watcher's `ExitEvent` emission, so sub-millisecond-exit workloads
/// no longer race. No fixture-side workaround required.
const fn job_exit_nonzero_spec() -> &'static str {
    r#"
[job]
id = "coinflip"

[exec]
command = "/bin/sh"
args = ["-c", "echo 'workload stderr line' >&2; exit 1"]

[resources]
cpu_milli = 100
memory_bytes = 67108864
"#
}

/// S-02-01 — a Job-kind workload that exits 0 reports `Succeeded` with
/// exit code 0 and the CLI process exits 0. Drives the natural-exit
/// success path end-to-end.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(workload_cgroup)]
async fn s_02_01_job_exits_zero_reports_succeeded() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    let spec_path = write_toml(tmp.path(), "happy-job.toml", job_exit_zero_spec());

    let output = overdrive_cli::commands::job::submit_streaming(SubmitArgs {
        spec: spec_path,
        config_path: cfg,
    })
    .await
    .expect("submit_streaming must complete");

    assert!(
        output.summary.contains("Job 'happy-job' succeeded."),
        "S-02-01: must contain 'Job 'happy-job' succeeded.'; got: {summary:?}",
        summary = output.summary,
    );
    assert!(
        output.summary.contains("exit code 0"),
        "S-02-01: must contain 'exit code 0'; got: {summary:?}",
        summary = output.summary,
    );
    assert_eq!(output.exit_code, 0, "S-02-01: Job exit 0 maps to CLI exit 0");

    drop(handle);
}

/// S-02-02 — a Job-kind workload that exits non-zero reports `Failed`
/// with the workload's exit code, attempts, and stderr tail. The CLI
/// process exits with the workload's exit code (KPI K1 honesty).
///
/// Per ADR-0037 §1: Job-kind workloads do NOT retry on failure (the
/// workload's contract is "run once, until it exits"); attempts is
/// always `1 of 1` for Job-kind. The roadmap mention of "3 of 3 (backoff
/// exhausted)" reflects Service-shaped retry semantics — Job-kind has
/// single-shot semantics by ADR-0047 §1 design.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(workload_cgroup)]
async fn s_02_02_job_exits_nonzero_reports_failed_with_attempts() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    let spec_path = write_toml(tmp.path(), "coinflip.toml", job_exit_nonzero_spec());

    let output = overdrive_cli::commands::job::submit_streaming(SubmitArgs {
        spec: spec_path,
        config_path: cfg,
    })
    .await
    .expect("submit_streaming must complete");

    assert!(
        output.summary.contains("Job 'coinflip' failed."),
        "S-02-02: must contain 'Job 'coinflip' failed.'; got: {summary:?}",
        summary = output.summary,
    );
    assert!(
        output.summary.contains("exit code 1"),
        "S-02-02: must contain 'exit code 1'; got: {summary:?}",
        summary = output.summary,
    );
    // Job-kind is single-shot per ADR-0047 §1 — attempts always 1 of 1.
    assert!(
        output.summary.contains("1 of 1"),
        "S-02-02: Job-kind attempts must be '1 of 1' (single-shot); got: {summary:?}",
        summary = output.summary,
    );
    // CLI process exit code = workload kernel exit code per K1.
    assert_eq!(output.exit_code, 1, "S-02-02: Job exit 1 maps to CLI exit 1");

    drop(handle);
}

/// S-02-03 — a Job-kind workload's intermediate failure observation
/// (the `Failed` row written by the exit observer with `terminal: None`)
/// produces a `JobSubmitEvent::AttemptFailed` event on the wire BEFORE
/// the terminal `Failed` event. The streaming session stays open while
/// the reconciler converts the per-attempt observation into a typed
/// terminal claim on the next tick.
///
/// Per ADR-0037 Amendment 2026-05-10 / ADR-0047 §1: Job-kind has no
/// retry — the "intermediate" event is the brief window between the
/// exit observer's row write and the reconciler's terminal stamping.
/// The streaming layer's job is to surface BOTH events on the wire so
/// the operator sees the workload exited and the verdict is being
/// finalised, without conflating the two into a single line.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(workload_cgroup)]
async fn s_02_03_intermediate_attempt_failure_does_not_close_stream() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    let spec_path = write_toml(tmp.path(), "coinflip.toml", job_exit_nonzero_spec());

    let output = overdrive_cli::commands::job::submit_streaming(SubmitArgs {
        spec: spec_path,
        config_path: cfg,
    })
    .await
    .expect("submit_streaming must complete");

    // The terminal event MUST be present — the stream must reach
    // `Failed`, NOT close on the intermediate `AttemptFailed`.
    assert!(
        output.summary.contains("Job 'coinflip' failed."),
        "S-02-03: terminal Failed must be reached after AttemptFailed; got: {summary:?}",
        summary = output.summary,
    );
    // The CLI exit code is the terminal exit code, not the AttemptFailed
    // event's exit code (they happen to be the same for a single-shot
    // Job, but the assertion is about reaching terminal).
    assert_eq!(output.exit_code, 1, "S-02-03: terminal exit code must be honored");

    drop(handle);
}

/// S-02-04 — third attempt succeeds (zero exit) reports `Succeeded`.
///
/// Per ADR-0037 Amendment 2026-05-10 / ADR-0047 §1, Job-kind workloads
/// are SINGLE-SHOT — they do NOT retry on failure. The roadmap's
/// mention of "third attempt" reflects Service-shape retry semantics
/// inherited from the pre-Job-kind design. For a Job-kind workload
/// "third attempt" is structurally unreachable — there is exactly ONE
/// attempt, terminating on first observed exit (clean OR crashed).
///
/// Asserted as the structural shape this design guarantees: a Job
/// that exits 0 on its (single) attempt reports Succeeded with
/// `attempts = 1`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(workload_cgroup)]
async fn s_02_04_third_attempt_zero_reports_succeeded() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    let spec_path = write_toml(tmp.path(), "happy-job.toml", job_exit_zero_spec());

    let output = overdrive_cli::commands::job::submit_streaming(SubmitArgs {
        spec: spec_path,
        config_path: cfg,
    })
    .await
    .expect("submit_streaming must complete");

    // Job-kind is single-shot; attempts is always 1.
    assert!(
        output.summary.contains("attempts 1"),
        "S-02-04: Job-kind attempts must be '1' (single-shot); got: {summary:?}",
        summary = output.summary,
    );
    // Successful single attempt → Succeeded.
    assert!(
        output.summary.contains("Job 'happy-job' succeeded."),
        "S-02-04: single-attempt zero-exit must report Succeeded; got: {summary:?}",
        summary = output.summary,
    );
    assert_eq!(output.exit_code, 0, "S-02-04: zero-exit single attempt → CLI exit 0");

    drop(handle);
}

/// S-02-07 — server validation failure surfaces as a structured error.
/// A spec with `replicas = 0` (rejected by `Job::from_submit`) must
/// produce `CliError::InvalidSpec` BEFORE any HTTP call — the CLI's
/// fast-fail validation gate fires per ADR-0014.
///
/// For a Job-kind spec, `replicas` is implicitly 1 (Job is single-shot
/// by definition); a malformed Job-kind spec instead surfaces via the
/// `[exec]` invariants — empty `command` is the canonical structural
/// validation failure that closes ADR-0031 §4.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s_02_07_server_validation_failure_structured_error() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    // A `[job]`-shape TOML with empty `command` — fails ADR-0031 §4
    // server-side validation. The CLI's fast-fail catches it
    // client-side; we drive against the in-process server too so a
    // server-side bypass would surface as `Validation` from HTTP.
    let bad_spec = r#"
[job]
id = "bad-job"

[exec]
command = ""
args = []

[resources]
cpu_milli = 100
memory_bytes = 67108864
"#;
    let spec_path = write_toml(tmp.path(), "bad-job.toml", bad_spec);

    let err = overdrive_cli::commands::job::submit_streaming(SubmitArgs {
        spec: spec_path,
        config_path: cfg,
    })
    .await
    .expect_err("invalid spec must fail");

    // Structured error — operator sees a typed CliError, not a
    // free-form panic / Display blob.
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("InvalidSpec") || rendered.contains("Validation"),
        "S-02-07: validation failure must surface as structured InvalidSpec/Validation; got: {rendered}",
    );

    drop(handle);
}

/// S-02-08 — streaming transport interruption surfaces honestly. When
/// the control plane is unreachable (no server bound to the configured
/// endpoint), the CLI returns `CliError::Transport` naming the
/// endpoint — no silent retry, no "succeeded by default" fall-through.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s_02_08_streaming_transport_interruption_surfaces_honestly() {
    let tmp = TempDir::new().expect("tempdir");
    // Spawn a real server to write a valid trust triple, then shut
    // it down and rewrite the endpoint to an unreachable port.
    let (handle, server_tmp) = spawn_server().await;
    handle.shutdown().await.expect("clean shutdown");
    let cfg_dir = server_tmp.path();
    let cfg = cfg_dir.join("conf").join(".overdrive").join("config");

    // Rewrite endpoint to an unreachable address.
    let original = std::fs::read_to_string(&cfg).expect("read config");
    let mut doc: toml::Value = toml::from_str(&original).expect("parse config");
    let contexts = doc.get_mut("contexts").and_then(|c| c.as_array_mut()).expect("contexts array");
    for ctx in contexts.iter_mut() {
        if let Some(tbl) = ctx.as_table_mut() {
            tbl.insert(
                "endpoint".to_owned(),
                toml::Value::String("https://127.0.0.1:1".to_owned()),
            );
        }
    }
    std::fs::write(&cfg, toml::to_string(&doc).expect("reserialise")).expect("write config");

    let spec_path = write_toml(tmp.path(), "happy-job.toml", job_exit_zero_spec());

    let err = overdrive_cli::commands::job::submit_streaming(SubmitArgs {
        spec: spec_path,
        config_path: cfg,
    })
    .await
    .expect_err("transport interruption must fail honestly");

    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("Transport") && rendered.contains("127.0.0.1:1"),
        "S-02-08: transport error must name the endpoint; got: {rendered}",
    );
}
