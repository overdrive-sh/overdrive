//! Regression test for `fix-cli-endpoint-config-only`.
//!
//! Before this fix, `--endpoint` carried a clap `default_value` of
//! `http://127.0.0.1:7001`, which made `args.endpoint` always set, which
//! made every handler pass `Some(args.endpoint.as_str())` to
//! `ApiClient::from_config_with_endpoint`, which short-circuited the
//! config-file fallback. The operator config at `~/.overdrive/config`
//! was never consulted for endpoint resolution — the scheme (`http` vs
//! `https`) in the error message was the smoking gun.
//!
//! The fix removes the override surface entirely (no `--endpoint`, no
//! `OVERDRIVE_ENDPOINT`). The operator config is the sole source of
//! the client endpoint.
//!
//! This test pins that contract: stand up a real in-process TLS server
//! on an ephemeral port, rewrite the operator config so its `endpoint`
//! field names that ephemeral port, invoke `job::submit` without any
//! endpoint argument (because the field no longer exists), and assert
//! the POST reaches the server — proving the client read the endpoint
//! from the config rather than from a hardcoded default.
//!
//! Per `crates/overdrive-cli/CLAUDE.md` this is a direct handler call,
//! not a subprocess.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use overdrive_cli::commands::job::{SubmitArgs, SubmitOutput};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use tempfile::TempDir;

/// Spin up a real in-process control-plane server on `127.0.0.1:0`.
/// Returns the handle and the backing `TempDir`; the `TempDir` must be
/// kept alive for the duration of the test — dropping it deletes the
/// trust-triple config.
async fn spawn_server() -> (ServeHandle, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let args = ServeArgs { bind, data_dir: tmp.path().to_path_buf() };
    let handle = overdrive_cli::commands::serve::run(args).await.expect("serve::run");
    (handle, tmp)
}

/// Path of the trust-triple config written by `serve::run` into
/// `<data_dir>/.overdrive/config`.
fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join(".overdrive").join("config")
}

/// Rewrite the `endpoint` field in the on-disk trust-triple TOML so it
/// names the real ephemeral port the server bound to. `serve::run`
/// records the *requested* bind (`https://127.0.0.1:0`), not the
/// resolved port — the operator-facing path goes through
/// `cluster::init` which records the default `https://127.0.0.1:7001`.
/// In this test we simulate the operator-flow end-state: the config
/// points at the real server.
fn rewrite_config_endpoint(config_path: &Path, new_endpoint: &str) {
    let original = std::fs::read_to_string(config_path).expect("read existing trust-triple config");
    let mut doc: toml::Value = toml::from_str(&original).expect("parse existing config toml");
    let contexts =
        doc.get_mut("contexts").and_then(|c| c.as_array_mut()).expect("contexts array present");
    for ctx in contexts.iter_mut() {
        if let Some(tbl) = ctx.as_table_mut() {
            tbl.insert("endpoint".to_owned(), toml::Value::String(new_endpoint.to_owned()));
        }
    }
    let rewritten = toml::to_string(&doc).expect("reserialise config toml");
    std::fs::write(config_path, rewritten).expect("write rewritten config");
}

fn write_valid_payments_toml(dir: &Path) -> PathBuf {
    let spec = r#"
id = "payments"
replicas = 3
cpu_milli = 500
memory_bytes = 536870912
"#;
    let path = dir.join("payments.toml");
    std::fs::write(&path, spec).expect("write payments.toml");
    path
}

/// The fix contract: when the operator config names the server's
/// endpoint, `job::submit` — invoked WITHOUT any endpoint argument —
/// reads that endpoint from the config and the POST reaches the server.
///
/// This test fails to compile against the pre-fix code (`SubmitArgs`
/// still has an `endpoint: Url` field). That is the RED-phase signal:
/// the type the production code accepts no longer has an `endpoint`
/// field because there is no override surface. Landing the GREEN-phase
/// changes to `SubmitArgs` is what makes this test compile; the
/// handler-level correctness is what makes it pass.
#[tokio::test]
async fn job_submit_reads_endpoint_from_config_when_no_override_is_provided() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    // Rewrite the config so its `endpoint` field names the actual
    // ephemeral port the server bound — mirroring the operator flow
    // where `cluster init` writes the endpoint and subsequent commands
    // read it. The pre-fix behaviour would ignore this and fall back
    // to the clap default (`http://127.0.0.1:7001`), which would miss
    // the live server entirely.
    let port = handle.endpoint().port().expect("endpoint port");
    let live_endpoint = format!("https://localhost:{port}");
    rewrite_config_endpoint(&cfg, &live_endpoint);

    let spec_path = write_valid_payments_toml(tmp.path());
    let args = SubmitArgs { spec: spec_path, config_path: cfg };

    let output: SubmitOutput =
        overdrive_cli::commands::job::submit(args).await.expect("job::submit");

    // The POST reached the server: the server assigned `job_id`
    // `payments` and a non-zero Raft commit index.
    assert_eq!(output.job_id, "payments", "SubmitOutput.job_id must be 'payments'");
    assert!(
        output.commit_index >= 1,
        "SubmitOutput.commit_index must be >= 1; got {}",
        output.commit_index,
    );

    // The resolved endpoint MUST be the one recorded in the config —
    // proving the client read it from disk rather than from a hardcoded
    // default. The scheme is `https`, not the pre-fix `http` default.
    assert_eq!(
        output.endpoint.as_str(),
        format!("{live_endpoint}/"),
        "SubmitOutput.endpoint must echo the endpoint recorded in the operator config",
    );

    handle.shutdown().await.expect("clean shutdown");
}
