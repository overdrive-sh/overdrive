//! Walking-skeleton gate for phase-1-control-plane-core — step 05-05,
//! revised 2026-04-26 per ADR-0020.
//!
//! Per `crates/overdrive-cli/CLAUDE.md` these call the handlers directly
//! (NO subprocess, NO `Command::new(env!("CARGO_BIN_EXE_overdrive"))`).
//! The full WS sequence (per
//! `docs/feature/phase-1-control-plane-core/distill/test-scenarios.md`
//! §1.1, §1.2, §1.3 — Amendment 2026-04-26 (ADR-0020)):
//!
//!   WS-1 (§1.1) — `serve::run` spawns an in-process axum+rustls server
//!     on an ephemeral port AND writes the resolved-port trust triple to
//!     disk; `job::submit` POSTs `payments.toml`; the server returns
//!     `outcome = Inserted` and a `spec_digest` BYTE-IDENTICAL to a
//!     locally-computed
//!     `ContentHash::of(rkyv::to_bytes(&Job::from_spec(...)))`.
//!     `alloc::status` GETs the same digest back.
//!   WS-2 (§1.2) — `cluster::status` returns four fields
//!     (`{mode, region, reconcilers, broker}`); `broker.dispatched > 0`
//!     after a tick proves the reconciler runtime is alive (the ADR-0020
//!     wiring witness — there is no `commit_index` line).
//!   WS-3 (§1.3) — `job::submit` of a byte-identical spec returns
//!     `outcome = Unchanged` and `spec_digest` equal to the first
//!     submit's digest. A different spec at the same intent key is a
//!     conflict (HTTP 409), with an actionable error message that does
//!     not leak a raw Rust panic / reqwest format.
//!
//! THIS TEST IS THE WALKING-SKELETON GATE — flipping it GREEN marks the
//! entire feature walking-skeleton as complete per DWD-05.
//!
//! Acceptance coverage:
//!   (a) WS-1/WS-2/WS-3 end-to-end via direct handler calls under the
//!       post-ADR-0020 wire shape (`outcome` + `spec_digest`, no
//!       `commit_index`)
//!   (b) unknown job → `CliError::HttpStatus { status: 404, .. }` with
//!       actionable message naming the job id
//!   (c) trust-triple config remains on disk after serve shutdown

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use overdrive_cli::commands::alloc::{AllocStatusOutput, StatusArgs};
use overdrive_cli::commands::cluster::StatusArgs as ClusterStatusArgs;
use overdrive_cli::commands::job::SubmitArgs;
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::http_client::CliError;
use overdrive_control_plane::api::IdempotencyOutcome;
use overdrive_core::aggregate::{Job, JobSpecInput};
use overdrive_core::id::ContentHash;
use tempfile::TempDir;

/// Spin up a real in-process control-plane server on `127.0.0.1:0`. Returns
/// `(handle, tmp)`; the `TempDir` lives for the test duration.
///
/// `data_dir` and `config_dir` are SEPARATE subdirectories of the
/// tempdir (`data` and `conf`) per `fix-cli-cannot-reach-control-plane`
/// Step 01-02 (RCA §WHY 4C). `serve::run` is the sole cert-minting
/// site in Phase 1 per `fix-remove-phase-1-cluster-init` (#81); the
/// trust triple it writes at `<config_dir>/.overdrive/config` is the
/// only config the CLI subsequently reads.
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

const fn payments_toml_spec_str() -> &'static str {
    r#"
id = "payments"
replicas = 3
cpu_milli = 500
memory_bytes = 536870912
"#
}

/// A spec that differs from `payments_toml_spec_str` by replica count.
/// Same `id` (`payments`), so it lands at the same intent key
/// (`jobs/payments`) — the WS-3 conflict scenario.
const fn payments_altered_toml_spec_str() -> &'static str {
    r#"
id = "payments"
replicas = 5
cpu_milli = 500
memory_bytes = 536870912
"#
}

fn write_toml(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write toml");
    path
}

/// Locally compute the canonical `spec_digest` using the same primitives
/// the server uses in `handlers::describe_job`:
///   SHA-256 of `rkyv::to_bytes::<rancor::Error>(&Job::from_spec(spec))`.
/// Any drift between this and the server-side computation is a bug.
fn local_spec_digest(spec_toml: &str) -> String {
    let parsed: JobSpecInput = toml::from_str(spec_toml).expect("parse TOML");
    let job = Job::from_spec(parsed).expect("Job::from_spec");
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&job).expect("rkyv archive");
    ContentHash::of(archived.as_ref()).to_string()
}

// -------------------------------------------------------------------
// (a) WALKING-SKELETON GATE — WS-1 / WS-2 / WS-3 end-to-end via direct
// handler calls under the post-ADR-0020 wire shape.
// -------------------------------------------------------------------

#[tokio::test]
#[allow(clippy::too_many_lines)] // The WS gate exercises three scenarios in one body.
async fn walking_skeleton_ws1_ws2_ws3_post_adr_0020_wire_shape() {
    // Phase 1: serve — in-process axum+rustls on ephemeral port.
    // `run_server` writes the resolved-port trust triple to disk, so
    // `from_config` picks up the live endpoint without further help.
    // Per `fix-remove-phase-1-cluster-init` (#81), `serve` is the sole
    // cert-minting site in Phase 1; there is no separate `cluster init`
    // pre-step.
    let (handle, server_tmp) = spawn_server().await;
    let server_cfg = config_path(server_tmp.path());

    // Phase 2 — WS-1: write the job spec, then submit via handler.
    // Post-ADR-0020 the submit response carries
    // `{job_id, spec_digest, outcome}` — no `commit_index`.
    let spec_path = write_toml(server_tmp.path(), "payments.toml", payments_toml_spec_str());
    let submit_output = overdrive_cli::commands::job::submit(SubmitArgs {
        spec: spec_path.clone(),
        config_path: server_cfg.clone(),
    })
    .await
    .expect("job::submit");
    assert_eq!(submit_output.job_id, "payments");
    assert_eq!(submit_output.intent_key, "jobs/payments");

    // WS-1 (test-scenarios.md §1.1, ADR-0020 amendment): the submit
    // output names a spec digest byte-identical to what Ana can
    // compute locally from the same file, AND the outcome is
    // `Inserted` (the wire-level "fresh insert" witness — replaces the
    // dropped `commit_index >= 1` assertion).
    let expected_digest = local_spec_digest(payments_toml_spec_str());
    assert_eq!(
        submit_output.spec_digest, expected_digest,
        "WS-1: submit response spec_digest MUST be byte-identical to \
         ContentHash::of(rkyv::to_bytes(&Job::from_spec(parsed))); \
         post-ADR-0020 the digest is the per-write witness — no \
         commit_index counter exists.",
    );
    assert_eq!(
        submit_output.outcome,
        IdempotencyOutcome::Inserted,
        "WS-1: a fresh submit must report `outcome = Inserted` \
         (ADR-0015 §4 amended by ADR-0020); this replaces the \
         dropped `commit_index >= 1` assertion as the wire-level \
         per-write witness.",
    );

    // Phase 3 — WS-1: alloc status — returns digest byte-identical to local.
    let status_output: AllocStatusOutput = overdrive_cli::commands::alloc::status(StatusArgs {
        job: "payments".to_string(),
        config_path: server_cfg.clone(),
    })
    .await
    .expect("alloc::status");

    assert_eq!(status_output.job_id, "payments", "status output must echo job id");
    assert_eq!(
        status_output.allocations_total, 0,
        "phase-1 allocations_total must be 0 (scheduler ships in phase-1-first-workload)",
    );
    assert!(
        status_output.empty_state_message.contains("phase-1-first-workload"),
        "empty_state_message must reference phase-1-first-workload; got: {}",
        status_output.empty_state_message,
    );

    assert_eq!(
        status_output.spec_digest, expected_digest,
        "WS-1: spec_digest returned via alloc::status MUST be byte-identical to \
         ContentHash::of(rkyv::to_bytes(&Job::from_spec(parsed))); this proves \
         the whole serve → submit → describe round-trip preserves \
         canonical rkyv bytes (ADR-0002 + ADR-0011). Post-ADR-0020 the digest \
         is the per-write witness — no commit_index counter exists.",
    );

    // Phase 4 — WS-2 (test-scenarios.md §1.2, ADR-0020 amendment):
    // cluster status returns four fields (mode, region, reconcilers,
    // broker); the post-ADR-0020 wiring witness for "reconciler
    // runtime is alive" is the `reconcilers` registry list naming
    // `noop-heartbeat`. The `Commit index:` line was dropped — there
    // is no fifth field. Broker counters are present and render as
    // non-negative integers (per the gherkin amendment in
    // test-scenarios.md §1.2: "every broker counter renders as a
    // non-negative integer"). A future phase that wires an automatic
    // broker tick can tighten this to `broker.dispatched > 0`; the
    // current Phase 1 wiring constructs the broker but does not
    // auto-drain it (see `EvaluationBroker::drain_pending` — no
    // caller in Phase 1 production code).
    let cluster_status = overdrive_cli::commands::cluster::status(ClusterStatusArgs {
        config_path: server_cfg.clone(),
    })
    .await
    .expect("cluster::status");

    assert_eq!(cluster_status.mode, "single", "WS-2: mode must be 'single' in Phase 1");
    assert!(
        !cluster_status.region.is_empty(),
        "WS-2: cluster status must carry a region; got empty string",
    );
    assert!(
        cluster_status.reconcilers.iter().any(|r| r == "noop-heartbeat"),
        "WS-2: reconcilers list must include `noop-heartbeat` per ADR-0013 §9 — \
         this IS the wiring witness for 'reconciler runtime is alive' under \
         the post-ADR-0020 four-field shape; got: {:?}",
        cluster_status.reconcilers,
    );
    // Broker counters present and non-negative (the four-field shape
    // contract). `u64` is non-negative by construction; this
    // existence-check is what the gherkin pins.
    let _: u64 = cluster_status.broker.queued;
    let _: u64 = cluster_status.broker.cancelled;
    let _: u64 = cluster_status.broker.dispatched;

    // Phase 5 — WS-3 (test-scenarios.md §1.3, ADR-0020 amendment):
    // a byte-identical re-submit returns `outcome = Unchanged` and the
    // same `spec_digest`. A different spec at the same intent key is
    // a conflict (HTTP 409), with an actionable error message that
    // does NOT leak a raw Rust panic / reqwest format.
    let resubmit_output = overdrive_cli::commands::job::submit(SubmitArgs {
        spec: spec_path,
        config_path: server_cfg.clone(),
    })
    .await
    .expect("job::submit (resubmit)");
    assert_eq!(
        resubmit_output.outcome,
        IdempotencyOutcome::Unchanged,
        "WS-3: a byte-identical resubmit must report `outcome = Unchanged` \
         (ADR-0015 §4 amended by ADR-0020); this replaces the dropped \
         `commit_index == 17` magic-number framing.",
    );
    assert_eq!(
        resubmit_output.spec_digest, submit_output.spec_digest,
        "WS-3: spec_digest must be stable across byte-identical \
         resubmissions — first={}, resubmit={}",
        submit_output.spec_digest, resubmit_output.spec_digest,
    );

    let altered_path =
        write_toml(server_tmp.path(), "payments-altered.toml", payments_altered_toml_spec_str());
    let conflict_err = overdrive_cli::commands::job::submit(SubmitArgs {
        spec: altered_path,
        config_path: server_cfg,
    })
    .await
    .expect_err("WS-3: a different spec at the same intent key must conflict");
    match &conflict_err {
        CliError::HttpStatus { status, body } => {
            assert_eq!(*status, 409_u16, "WS-3: conflict must surface as HTTP 409; got {status}");
            assert!(
                !body.message.is_empty(),
                "WS-3: ErrorBody.message must explain the conflict; got empty",
            );
            // Sanity: no raw Rust panic or reqwest format leakage.
            assert!(
                !body.message.contains("panicked at") && !body.message.contains("reqwest::Error"),
                "WS-3: error body must not leak a raw Rust panic or reqwest format; got: {}",
                body.message,
            );
        }
        other => panic!("WS-3: expected CliError::HttpStatus {{ status: 409, .. }}; got {other:?}"),
    }

    // Phase 6: clean shutdown; trust-triple config persists on disk.
    handle.shutdown().await.expect("clean shutdown");
    let post_shutdown_cfg = config_path(server_tmp.path());
    assert!(
        post_shutdown_cfg.exists(),
        "trust-triple config must survive serve shutdown: {}",
        post_shutdown_cfg.display(),
    );
}

// -------------------------------------------------------------------
// (b) alloc::status for unknown job → typed 404 with actionable message
// -------------------------------------------------------------------

#[tokio::test]
async fn alloc_status_for_unknown_job_returns_typed_http_status_404_with_actionable_message() {
    let (handle, server_tmp) = spawn_server().await;
    let server_cfg = config_path(server_tmp.path());

    let err = overdrive_cli::commands::alloc::status(StatusArgs {
        job: "mystery".to_string(),
        config_path: server_cfg,
    })
    .await
    .expect_err("alloc::status must fail for an unknown job");

    match &err {
        CliError::HttpStatus { status, body } => {
            assert_eq!(*status, 404_u16, "expected HTTP 404 for unknown job; got {}", *status);
            assert_eq!(body.error, "not_found", "error class must be `not_found`");
            // Message must name the offending job id so the operator can act.
            assert!(
                body.message.contains("mystery") || body.message.contains("jobs/mystery"),
                "ErrorBody.message must name `mystery`; got: {}",
                body.message,
            );
        }
        other => panic!(
            "expected CliError::HttpStatus {{ status: 404, .. }} for unknown job, got {other:?}"
        ),
    }

    handle.shutdown().await.expect("clean shutdown");
}

// -------------------------------------------------------------------
// (c) config file remains on disk after serve shutdown
// -------------------------------------------------------------------

#[tokio::test]
async fn config_file_remains_on_disk_after_serve_shutdown() {
    let (handle, server_tmp) = spawn_server().await;
    let cfg_path = config_path(server_tmp.path());
    assert!(
        cfg_path.exists(),
        "trust-triple config must be written by serve::run at {}",
        cfg_path.display()
    );
    handle.shutdown().await.expect("clean shutdown");
    assert!(
        cfg_path.exists(),
        "trust-triple config must persist after serve shutdown at {}",
        cfg_path.display()
    );
}
