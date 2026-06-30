//! Direct-handler-call acceptance tests for `overdrive workload restart`
//! — backend-instance-replacement slice 01, step 01-04 (the e2e
//! production-loop closer).
//!
//! Per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no
//! subprocess" these call the CLI handler
//! (`commands::workload::restart`) DIRECTLY as a Rust function against a
//! real in-process control-plane server (`commands::serve::run_with_dataplane`
//! on an ephemeral port; the trust triple is written by `serve`). NO
//! `Command::spawn`, NO `CARGO_BIN_EXE_overdrive` — the `@real-io` proof
//! for the restart route rides the production
//! `POST /v1/jobs/:id/restart` → real `LocalIntentStore` path in
//! `run_server`, not a handler-internal pipeline substitute (per
//! `docs/analysis/rca-user-port-gap.md`).
//!
//! Acceptance coverage:
//!   * S-BIR-CLI-RESTART-SUCCESS (US-BIR-1, the e2e production loop): a
//!     declared workload `payments` with NO `/stop` sentinel is restarted
//!     via the handler; per the 01-03 handler contract (absent `/stop` at
//!     the check-exists read ⇒ `Restarted`) the outcome is DETERMINISTIC,
//!     so the handler returns
//!     `Ok(RestartOutput { workload_id: "payments", outcome: Restarted })`.
//!     The deterministic `Restarted` assertion kills a mutation that
//!     hardcodes `outcome: Resumed`.
//!   * S-BIR-CLI-RESTART-RESUMED (US-BIR-1, the other outcome direction):
//!     a declared workload `payments` is STOPPED through the production
//!     stop verb (writing the `/stop` sentinel), then restarted; the
//!     restart's check-exists read finds a PRESENT `/stop` ⇒ deterministic
//!     `Resumed`. Together with the success scenario this pins BOTH
//!     outcome directions, so neither hardcode (`Restarted` nor `Resumed`)
//!     can survive — proving the CLI handler PRESERVES the server's label
//!     (`outcome: resp.outcome`) rather than fabricating it.
//!   * S-BIR-CLI-RESTART-UNKNOWN (US-BIR-1 AC5): restarting a workload
//!     that was never declared returns
//!     `Err(CliError::HttpStatus { status: 404, body.error == "not_found" })`
//!     AND `render::cli_error_to_exit_code(&err)` is non-zero — the CLI
//!     maps the handler 404 to an honest typed error → a non-zero exit
//!     code, not a silent success.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use overdrive_cli::commands::deploy::{DeployArgs, DeployOutput, StopArgs};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::commands::workload::{RestartArgs, RestartOutput};
use overdrive_cli::http_client::CliError;
use overdrive_control_plane::api::RestartOutcome;
use tempfile::TempDir;

/// Spin up a real in-process control-plane server on `127.0.0.1:0`.
/// Returns the handle and the backing `TempDir`; the `TempDir` must be
/// kept alive for the duration of the test — dropping it deletes the
/// trust-triple config.
///
/// Mirrors the canonical shape in `deploy.rs` / `endpoint_from_config.rs`:
/// `data_dir` and `config_dir` are SEPARATE subdirectories of the
/// tempdir per `fix-cli-cannot-reach-control-plane` Step 01-02.
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
        // Hermetic in-process boot KEK.
        std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
    )
    .await
    .expect("serve::run");
    (handle, tmp)
}

/// Path of the trust-triple config written by `serve::run` into
/// `<config_dir>/.overdrive/config` — given the tempdir root from
/// [`spawn_server`].
fn config_path(tmp: &Path) -> PathBuf {
    tmp.join("conf").join(".overdrive").join("config")
}

/// Write a valid `payments` workload TOML and return its path.
fn write_valid_payments_toml(dir: &Path) -> PathBuf {
    let spec = r#"
id = "payments"
replicas = 3

[resources]
cpu_milli = 500
memory_bytes = 536870912

[exec]
command = "/bin/true"
args = []
"#;
    let path = dir.join("payments.toml");
    std::fs::write(&path, spec).expect("write payments.toml");
    path
}

// -------------------------------------------------------------------
// S-BIR-CLI-RESTART-SUCCESS — the e2e production loop: serve + deploy +
// workload restart, driven through the production route into the real
// in-process LocalIntentStore.
// -------------------------------------------------------------------

#[tokio::test]
async fn workload_restart_for_declared_workload_returns_restart_output() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    // Declare the workload through the production deploy path first, so
    // the restart's check-exists read finds it (otherwise it 404s).
    let spec_path = write_valid_payments_toml(tmp.path());
    let deployed: DeployOutput = overdrive_cli::commands::deploy::deploy(DeployArgs {
        spec: spec_path,
        config_path: cfg.clone(),
    })
    .await
    .expect("deploy::deploy must declare `payments`");
    assert_eq!(deployed.workload_id, "payments", "precondition: `payments` deployed");

    // Restart it via the new verb — the @real-io proof rides the
    // production POST /v1/jobs/payments/restart route into the real
    // LocalIntentStore in run_server.
    let output: RestartOutput = overdrive_cli::commands::workload::restart(RestartArgs {
        id: "payments".to_string(),
        config_path: cfg,
    })
    .await
    .expect("workload::restart must succeed for a declared workload");

    assert_eq!(
        output.workload_id, "payments",
        "RestartOutput.workload_id must echo the restarted workload id",
    );
    // The fixture deploys `payments` with NO `/stop` sentinel, so per the
    // 01-03 handler contract (absent `/stop` at the check-exists read ⇒
    // `Restarted`) the outcome is DETERMINISTIC. Asserting the exact value
    // — not `matches!(... | ...)` over the whole enum — kills a mutation
    // that hardcodes `outcome: RestartOutcome::Resumed` in the handler.
    assert_eq!(
        output.outcome,
        RestartOutcome::Restarted,
        "absent /stop sentinel ⇒ the CLI must preserve the server's Restarted label",
    );
    // The endpoint the POST was issued to must match the one the server
    // recorded in the trust triple — proving the handler read it from
    // disk, not a hardcoded default.
    assert_eq!(
        output.endpoint,
        *handle.endpoint(),
        "RestartOutput.endpoint must echo the endpoint recorded in the operator config",
    );

    handle.shutdown().await.expect("clean shutdown");
}

// -------------------------------------------------------------------
// S-BIR-CLI-RESTART-RESUMED — the OTHER outcome direction: deploy +
// stop (writing the /stop sentinel) + restart, driven through the
// production deploy/stop/restart routes into the real in-process
// LocalIntentStore. A PRESENT /stop at the restart's check-exists read
// ⇒ deterministic `Resumed`. Pins the hardcode-`Restarted` direction
// that the success scenario alone cannot kill.
// -------------------------------------------------------------------

#[tokio::test]
async fn workload_restart_of_stopped_workload_returns_resumed() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    // Declare the workload through the production deploy path.
    let spec_path = write_valid_payments_toml(tmp.path());
    let deployed: DeployOutput = overdrive_cli::commands::deploy::deploy(DeployArgs {
        spec: spec_path,
        config_path: cfg.clone(),
    })
    .await
    .expect("deploy::deploy must declare `payments`");
    assert_eq!(deployed.workload_id, "payments", "precondition: `payments` deployed");

    // Stop it through the SHIPPED production stop verb — this POSTs the
    // production POST /v1/jobs/payments/stop route, writing the `/stop`
    // sentinel into the real LocalIntentStore.
    let _stopped = overdrive_cli::commands::deploy::stop(StopArgs {
        id: "payments".to_string(),
        config_path: cfg.clone(),
    })
    .await
    .expect("stop must suspend the declared workload");

    // Restart it — the restart's check-exists read now finds a PRESENT
    // `/stop` sentinel ⇒ deterministic `Resumed`. Asserting the exact
    // value kills a mutation that hardcodes `outcome: Restarted`.
    let output: RestartOutput = overdrive_cli::commands::workload::restart(RestartArgs {
        id: "payments".to_string(),
        config_path: cfg,
    })
    .await
    .expect("restart of a stopped workload must succeed");

    assert_eq!(
        output.workload_id, "payments",
        "RestartOutput.workload_id must echo the restarted workload id",
    );
    assert_eq!(
        output.outcome,
        RestartOutcome::Resumed,
        "present /stop sentinel ⇒ the CLI must preserve the server's Resumed label",
    );

    handle.shutdown().await.expect("clean shutdown");
}

// -------------------------------------------------------------------
// S-BIR-CLI-RESTART-UNKNOWN — an undeclared workload maps to an honest
// typed 404 → a non-zero exit code, never a silent success.
// -------------------------------------------------------------------

#[tokio::test]
async fn workload_restart_for_unknown_workload_returns_typed_404_and_nonzero_exit() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    let err = overdrive_cli::commands::workload::restart(RestartArgs {
        id: "nonexistent".to_string(),
        config_path: cfg,
    })
    .await
    .expect_err("workload::restart must fail for an undeclared workload");

    match &err {
        CliError::HttpStatus { status, body } => {
            assert_eq!(*status, 404_u16, "expected HTTP 404 for unknown workload; got {}", *status);
            assert_eq!(body.error, "not_found", "error class must be `not_found`");
        }
        other => panic!(
            "expected CliError::HttpStatus (status: 404) for unknown workload, got {other:?}"
        ),
    }

    // The CLI maps the 404 to a non-zero exit code — not a silent
    // success. A mutation that swallows the 404 / exits 0 on an unknown
    // id must be killed here.
    let exit_code = overdrive_cli::render::cli_error_to_exit_code(&err);
    assert_ne!(exit_code, 0, "an unknown-workload restart must map to a non-zero exit code");

    handle.shutdown().await.expect("clean shutdown");
}
