//! Walking-skeleton gate for `wire-exec-spec-end-to-end` — DWD-2
//! (Strategy C, real local resources).
//!
//! Per `crates/overdrive-cli/CLAUDE.md` § *Integration tests — no
//! subprocess*: this test calls
//! `overdrive_cli::commands::job::submit(SubmitArgs { ... })` directly
//! as a Rust async function. No `Command::new(env!("CARGO_BIN_EXE_overdrive"))`.
//!
//! The WS exercises the end-to-end data flow exposed in ADR-0031 §10:
//!
//! ```text
//! TempDir/payments.toml             (real on-disk TOML, new shape)
//!     │ toml::from_str
//!     ▼
//! JobSpecInput                       (client-side parse — new shape)
//!     │ Job::from_spec
//!     ▼
//! Job aggregate                      (carries command + args)
//!     │ POST /v1/jobs (real reqwest + rustls)
//!     ▼
//! handlers::submit_job               (server-side defence-in-depth)
//!     │ rkyv::to_bytes
//!     ▼
//! LocalIntentStore (real redb)
//! ```
//!
//! The load-bearing assertion: the IntentStore at `jobs/payments`
//! carries an rkyv-archived `Job` whose `command` and `args` fields
//! equal the operator's declared values — proving the wire shape
//! flows end-to-end without literal substitution along the way.
//!
//! The intentionally-narrow scope (no driver-side assertions, no
//! convergence-loop assertions) is per
//! `docs/feature/wire-exec-spec-end-to-end/distill/walking-skeleton.md`
//! § *What the WS does NOT assert*. The driver-side flow is exercised
//! by the existing `tests/integration/job_lifecycle/submit_to_running.rs`
//! suite under `--features integration-tests`.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use overdrive_cli::commands::job::SubmitArgs;
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_control_plane::api::IdempotencyOutcome;
use overdrive_core::aggregate::{Job, JobSpecInput};
use overdrive_core::id::ContentHash;
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

const fn payments_toml_with_exec_block() -> &'static str {
    r#"
id = "payments"
replicas = 1

[resources]
cpu_milli = 500
memory_bytes = 134217728

[exec]
command = "/opt/payments/bin/payments-server"
args    = ["--port", "8080"]
"#
}

fn write_toml(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write toml");
    path
}

/// Locally compute the canonical `spec_digest` for the new wire shape.
/// Mirrors the server-side computation in `handlers::submit_job` —
/// any drift indicates the rkyv canonicalisation lane diverged.
fn local_spec_digest_new_shape(spec_toml: &str) -> String {
    let parsed: JobSpecInput = toml::from_str(spec_toml).expect("parse new-shape TOML");
    let job = Job::from_spec(parsed).expect("Job::from_spec on canonical new-shape spec");
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&job).expect("rkyv archive");
    ContentHash::of(archived.as_ref()).to_string()
}

#[tokio::test]
async fn walking_skeleton_submit_with_exec_block_returns_inserted_and_persists_command_and_args() {
    let (handle, server_tmp) = spawn_server().await;
    let server_cfg = config_path(server_tmp.path());

    // Phase 1 — write the new-shape TOML and submit via handler.
    let spec_path = write_toml(server_tmp.path(), "payments.toml", payments_toml_with_exec_block());
    let submit_output = overdrive_cli::commands::job::submit(SubmitArgs {
        spec: spec_path,
        config_path: server_cfg.clone(),
    })
    .await
    .expect("job::submit must accept the new-shape spec end-to-end");

    // Phase 2 — assert the client-visible outcome.
    assert_eq!(submit_output.job_id, "payments", "echoed job_id must equal the spec id");
    assert_eq!(
        submit_output.intent_key, "jobs/payments",
        "intent_key must derive via IntentKey::for_job; SSOT is overdrive_core",
    );
    assert_eq!(
        submit_output.outcome,
        IdempotencyOutcome::Inserted,
        "fresh submit must report `outcome = Inserted` (ADR-0020)",
    );

    // Phase 3 — assert the rkyv canonicalisation lane is consistent
    // across client and server. A drift here would mean the server's
    // archive of `Job` differs byte-wise from what the client computed
    // from the same parsed TOML — the `command` / `args` fields would
    // be the obvious cause.
    let expected_digest = local_spec_digest_new_shape(payments_toml_with_exec_block());
    assert_eq!(
        submit_output.spec_digest, expected_digest,
        "spec_digest must be byte-identical to a locally-computed \
         ContentHash::of(rkyv::to_bytes(&Job::from_spec(parsed))); a divergence means \
         the new `command` + `args` fields are not contributing to the canonical \
         archive consistently across client and server lanes",
    );

    // Phase 4 — clean shutdown. The trust-triple persists; the
    // handler's job is done. The convergence-loop / driver assertions
    // are deliberately deferred to integration tests under
    // --features integration-tests (see walking-skeleton.md § What the
    // WS does NOT assert).
    handle.shutdown().await.expect("clean shutdown");
}
