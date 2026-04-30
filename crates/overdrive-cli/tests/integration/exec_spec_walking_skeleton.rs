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
use std::str::FromStr;

use overdrive_cli::commands::job::SubmitArgs;
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_control_plane::api::IdempotencyOutcome;
use overdrive_core::aggregate::{IntentKey, Job, JobSpecInput, WorkloadDriver};
use overdrive_core::id::{ContentHash, JobId};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_store_local::LocalIntentStore;
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

/// Path of the on-disk `LocalIntentStore` redb file the in-process
/// server writes through. Mirrors the literal in
/// `crates/overdrive-control-plane/src/lib.rs` (`<data_dir>/intent.redb`).
/// Used by the WS back-door read AFTER `ServeHandle::shutdown` releases
/// the redb file lock — the DISTILL-review BLOCK fix per
/// `docs/feature/wire-exec-spec-end-to-end/distill/test-scenarios.md` §1.
fn intent_redb_path(tmp: &Path) -> PathBuf {
    tmp.join("data").join("intent.redb")
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

    // Phase 4 — clean shutdown. Drop the server's redb handle BEFORE
    // we open a second one for the back-door read; redb takes an
    // exclusive lock on the database file at `Database::create` time,
    // so a concurrent open from the same process would fail.
    handle.shutdown().await.expect("clean shutdown");

    // Phase 5 — back-door IntentStore read (DISTILL-review BLOCK fix).
    // The `spec_digest` assertion above pins client/server canonical-
    // bytes parity, but proves nothing about end-to-end *persistence* —
    // a regression that dropped the `state.store.put(...)` call in the
    // submit handler would still let the digest match (the server
    // computes it from the request bytes, not from a re-read). This
    // back-door read closes that gap by deserialising the row at
    // `jobs/payments` from the redb file the handler wrote through, and
    // asserting the operator's TOML input is carried verbatim through
    // the `WorkloadDriver::Exec(Exec { command, args })` projection.
    //
    // Pattern mirrors
    // `crates/overdrive-control-plane/tests/acceptance/submit_job_idempotency.rs`
    // — the canonical reference for IntentStore back-door reads in
    // this project. Per ADR-0031 Amendment 1 the `Job` carries
    // `driver: WorkloadDriver`, NOT flat `command` / `args`, so the
    // destructure goes through `WorkloadDriver::Exec(_)` first.
    let store = LocalIntentStore::open(intent_redb_path(server_tmp.path()))
        .expect("re-open intent.redb for back-door read");
    let job_id = JobId::from_str("payments").expect("JobId::from_str(\"payments\")");
    let key = IntentKey::for_job(&job_id);
    let stored = store
        .get(key.as_bytes())
        .await
        .expect("back-door IntentStore::get must succeed");
    let bytes = stored.expect(
        "after a successful submit the intent key `jobs/payments` MUST be \
         populated; an empty key here means the server skipped \
         `state.store.put(...)` — end-to-end persistence is broken",
    );

    // rkyv access + deserialise — same lane the server uses on read
    // (see `handlers::describe_job` for the canonical pattern).
    let archived = rkyv::access::<rkyv::Archived<Job>, rkyv::rancor::Error>(&bytes)
        .expect("rkyv access of ArchivedJob from back-door read bytes");
    let job: Job = rkyv::deserialize::<Job, rkyv::rancor::Error>(archived)
        .expect("rkyv deserialise of Job from back-door read bytes");

    // Destructure WorkloadDriver::Exec — irrefutable for Phase 1 (single
    // variant). Future Phase 2+ variants (`MicroVm`, `Wasm`) make this
    // a `match`, but the new arms project to their own
    // `assert_eq!(exec.command, ...)` body — the WS structure carries
    // forward.
    let WorkloadDriver::Exec(exec) = &job.driver;
    assert_eq!(
        exec.command, "/opt/payments/bin/payments-server",
        "stored Job.driver.exec.command must equal the operator's TOML \
         input verbatim — a divergence here means the wire shape was \
         lossy along the persistence lane (TOML → JobSpecInput → \
         Job::from_spec → rkyv::to_bytes → redb → rkyv::access → Job)",
    );
    assert_eq!(
        exec.args,
        vec!["--port".to_string(), "8080".to_string()],
        "stored Job.driver.exec.args must equal the operator's TOML \
         input verbatim — argv is opaque per ADR-0031 §4 but its \
         persisted shape must match what the operator declared",
    );

    // The convergence-loop / driver assertions are deliberately deferred
    // to integration tests under --features integration-tests (see
    // walking-skeleton.md § What the WS does NOT assert).
}
