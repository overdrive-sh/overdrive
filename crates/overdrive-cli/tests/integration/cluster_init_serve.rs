//! Integration tests for `overdrive_cli::commands::cluster::init` and
//! `overdrive_cli::commands::serve::run` — step 05-02.
//!
//! Per `crates/overdrive-cli/CLAUDE.md` these call the handlers directly
//! (NO subprocess). The handlers stand up a real in-process control-plane
//! server on an ephemeral port, probe it via the `ApiClient` from step
//! 05-01, and then exercise the `ServeHandle::shutdown()` cancellation
//! path.
//!
//! Acceptance coverage:
//!   (a) `cluster::init` writes a parseable TOML trust triple at
//!       `<config_dir>/.overdrive/config` (ADR-0019)
//!   (b) re-invoking `cluster::init` on existing config re-mints (CA
//!       bytes differ) per ADR-0010 §R4
//!   (c) `serve::run` binds an ephemeral port and the `ApiClient` probe
//!       through that port succeeds
//!   (d) `ServeHandle::shutdown` completes within a 5-second deadline
//!   (e) After shutdown, a fresh `ApiClient` probe returns
//!       `CliError::Transport`
//!   (f) `serve::run` bind failure on an occupied port maps to
//!       `CliError` with an actionable message

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use overdrive_cli::commands::cluster::{InitArgs, InitOutput};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::http_client::{ApiClient, CliError};
use tempfile::TempDir;

/// Read and extract the base64-encoded `ca` field from the trust-triple
/// TOML at `config_path`. Used to prove re-init re-mints with different
/// CA bytes (ADR-0010 §R4). Per ADR-0019, the file is TOML with
/// `[[contexts]]` as an array-of-tables keyed on `name`.
fn read_ca_bytes_from_config(config_path: &Path) -> Vec<u8> {
    use base64::Engine as _;
    let toml_str = std::fs::read_to_string(config_path).expect("read config toml");
    let doc: toml::Value = toml::from_str(&toml_str).expect("parse config toml");
    let contexts = doc.get("contexts").and_then(|c| c.as_array()).expect("contexts array");
    let local = contexts
        .iter()
        .find(|c| c.get("name").and_then(|n| n.as_str()) == Some("local"))
        .expect("local context present");
    let ca_b64 = local.get("ca").and_then(|v| v.as_str()).expect("ca field present as string");
    base64::engine::general_purpose::STANDARD.decode(ca_b64).expect("ca is valid base64")
}

/// Rewrite the `endpoint` field in the on-disk trust-triple TOML so it
/// names the real ephemeral port the server bound to. The operator
/// config is the sole source of the endpoint (no `--endpoint`
/// override), so tests mutate the on-disk config to point at the live
/// server.
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

/// Build an `ApiClient` for the live server bound on `bound` by first
/// rewriting the on-disk trust triple so its `endpoint` field names the
/// real ephemeral port, then loading through the sole `from_config`
/// constructor.
fn build_client(config_path: &Path, bound: SocketAddr) -> ApiClient {
    let live_endpoint = format!("https://localhost:{}", bound.port());
    rewrite_config_endpoint(config_path, &live_endpoint);
    ApiClient::from_config(config_path).expect("build ApiClient")
}

// -------------------------------------------------------------------
// (a) cluster::init writes a parseable TOML trust triple (ADR-0019)
// -------------------------------------------------------------------

#[tokio::test]
async fn cluster_init_writes_trust_triple_at_config_path() {
    let tmp = TempDir::new().expect("tempdir");
    let args = InitArgs { config_dir: Some(tmp.path().to_path_buf()), force: false };

    let output: InitOutput = overdrive_cli::commands::cluster::init(args).await.expect("init");

    let expected = tmp.path().join(".overdrive").join("config");
    assert_eq!(output.config_path, expected, "config_path must be <config_dir>/.overdrive/config");
    assert!(output.config_path.exists(), "trust-triple file must exist on disk");

    // Parseable TOML matching ADR-0019: `current-context = "local"`
    // plus an `[[contexts]]` array-of-tables where each entry carries
    // `name`, `endpoint`, `ca`, `crt`, `key`.
    let toml_str = std::fs::read_to_string(&output.config_path).expect("read config");
    let doc: toml::Value = toml::from_str(&toml_str).expect("valid TOML");
    assert_eq!(
        doc.get("current-context").and_then(|v| v.as_str()),
        Some("local"),
        "top-level `current-context` must be `\"local\"` per ADR-0019",
    );
    let contexts = doc.get("contexts").and_then(|c| c.as_array()).expect("contexts array present");
    let local = contexts
        .iter()
        .find(|c| c.get("name").and_then(|n| n.as_str()) == Some("local"))
        .expect("contexts entry with name = \"local\" must be present");
    assert!(local.get("ca").is_some(), "contexts[local].ca must exist");
    assert!(local.get("crt").is_some(), "contexts[local].crt must exist");
    assert!(local.get("key").is_some(), "contexts[local].key must exist");
    assert!(local.get("endpoint").is_some(), "contexts[local].endpoint must exist");
}

// -------------------------------------------------------------------
// (b) re-init re-mints (CA bytes differ per ADR-0010 §R4)
// -------------------------------------------------------------------

#[tokio::test]
async fn cluster_init_re_init_re_mints_with_different_ca_bytes() {
    let tmp = TempDir::new().expect("tempdir");

    let first = overdrive_cli::commands::cluster::init(InitArgs {
        config_dir: Some(tmp.path().to_path_buf()),
        force: false,
    })
    .await
    .expect("first init");

    let first_ca = read_ca_bytes_from_config(&first.config_path);

    // Second init against the same config_dir — per ADR-0010 §R4 this
    // MUST re-mint a fresh CA even though the config file already
    // exists (no --force required; Phase 1 reserves --force for future
    // non-destructive modes).
    let second = overdrive_cli::commands::cluster::init(InitArgs {
        config_dir: Some(tmp.path().to_path_buf()),
        force: false,
    })
    .await
    .expect("second init");

    let second_ca = read_ca_bytes_from_config(&second.config_path);

    assert_ne!(
        first_ca, second_ca,
        "re-init must re-mint CA: two consecutive init calls must produce distinct CA bytes per ADR-0010 §R4",
    );
}

// -------------------------------------------------------------------
// (c) serve::run binds ephemeral port; `ApiClient` probe succeeds
// -------------------------------------------------------------------

#[tokio::test]
async fn serve_run_binds_ephemeral_port_and_returns_serve_handle() {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");

    let args = ServeArgs { bind, data_dir: tmp.path().to_path_buf() };
    let handle: ServeHandle = overdrive_cli::commands::serve::run(args).await.expect("serve::run");

    // Ephemerally bound port must be non-zero.
    let endpoint = handle.endpoint();
    let port = endpoint.port().expect("endpoint must carry a port");
    assert_ne!(port, 0, "ephemeral port must not be zero: got {endpoint}");

    // `ApiClient` probe against the live server: /v1/nodes is the real
    // observation-read endpoint wired in step 03-03. A fresh store
    // returns {"rows":[]}.
    let config_path = tmp.path().join(".overdrive").join("config");
    let bound: SocketAddr = format!("127.0.0.1:{port}").parse().expect("parse bound addr");
    let client = build_client(&config_path, bound);
    let nodes = client.node_list().await.expect("node_list against live server");
    assert!(nodes.rows.is_empty(), "fresh store must report zero node rows");

    handle.shutdown().await.expect("clean shutdown");
}

// -------------------------------------------------------------------
// (d) shutdown completes within 5-second deadline
// -------------------------------------------------------------------

#[tokio::test]
async fn serve_handle_shutdown_completes_cleanly_within_5s_deadline() {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");

    let args = ServeArgs { bind, data_dir: tmp.path().to_path_buf() };
    let handle = overdrive_cli::commands::serve::run(args).await.expect("serve::run");

    let shutdown_fut = handle.shutdown();
    let timed: Result<_, tokio::time::error::Elapsed> =
        tokio::time::timeout(Duration::from_secs(5), shutdown_fut).await;
    let inner = timed.expect("shutdown did not complete within 5s deadline");
    inner.expect("shutdown returned error");
}

// -------------------------------------------------------------------
// (e) probe after shutdown returns CliError::Transport
// -------------------------------------------------------------------

#[tokio::test]
async fn probe_after_shutdown_returns_transport_error() {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");

    let args = ServeArgs { bind, data_dir: tmp.path().to_path_buf() };
    let handle = overdrive_cli::commands::serve::run(args).await.expect("serve::run");

    let port = handle.endpoint().port().expect("port");
    let config_path = tmp.path().join(".overdrive").join("config");
    let bound: SocketAddr = format!("127.0.0.1:{port}").parse().expect("parse bound addr");

    // Shut down FIRST, then build a fresh client and probe — the
    // server is gone.
    handle.shutdown().await.expect("clean shutdown");

    let client = build_client(&config_path, bound);
    let err = client.cluster_status().await.expect_err("probe after shutdown must fail");

    match &err {
        CliError::Transport { endpoint, .. } => {
            assert!(
                endpoint.contains(&port.to_string()),
                "Transport.endpoint must name the endpoint; got {endpoint}",
            );
        }
        other => panic!("expected CliError::Transport after shutdown, got {other:?}"),
    }
}

// -------------------------------------------------------------------
// Regression guard (bugfix fix-overdrive-config-path-doubled):
//
// Every test above passes `Some(tmp.path())` as `config_dir`, exercising
// only the explicit-override branch of `resolve_config_dir`. The HOME
// fallback branch — the one that actually runs in production when an
// operator runs `overdrive cluster init` with no flags — was previously
// untested, and that branch was writing the trust triple to
// `$HOME/.overdrive/.overdrive/config` (doubled `.overdrive` segment)
// instead of the ADR-0010 / ADR-0014 / ADR-0019 canonical
// `$HOME/.overdrive/config`.
//
// These two tests pin the HOME-fallback invariant:
//   (1) cluster::init writes to `$HOME/.overdrive/config` AND NOT to
//       `$HOME/.overdrive/.overdrive/config` (primary regression guard).
//   (2) the shared `default_operator_config_path` helper returns the
//       path that `cluster::init` actually writes — read and write
//       sites cannot drift again (structural invariant guard).
//
// Both mutate `$HOME` / `$OVERDRIVE_CONFIG_DIR` so they are serialised
// via `#[serial_test::serial(env)]`. Env mutation is `unsafe` on
// rustc 1.80+ (workspace `unsafe_op_in_unsafe_fn = deny`).
// -------------------------------------------------------------------

/// Save-and-restore guard for the two env vars the HOME-fallback branch
/// inspects. `Drop` restores the prior values on unwind, which keeps
/// env state sane when an assertion panics mid-test.
struct EnvGuard {
    prior_home: Option<std::ffi::OsString>,
    prior_config_dir: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn capture() -> Self {
        Self {
            prior_home: std::env::var_os("HOME"),
            prior_config_dir: std::env::var_os("OVERDRIVE_CONFIG_DIR"),
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: env mutation is process-wide and racy; `#[serial(env)]`
        // ensures no other test mutates $HOME / $OVERDRIVE_CONFIG_DIR
        // concurrently.
        unsafe {
            match &self.prior_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            match &self.prior_config_dir {
                Some(v) => std::env::set_var("OVERDRIVE_CONFIG_DIR", v),
                None => std::env::remove_var("OVERDRIVE_CONFIG_DIR"),
            }
        }
    }
}

#[tokio::test]
#[serial_test::serial(env)]
async fn resolve_config_dir_home_fallback_writes_at_canonical_path() {
    let tmp = TempDir::new().expect("tempdir");
    let _env = EnvGuard::capture();

    // SAFETY: `#[serial(env)]` prevents concurrent env mutation; the
    // `EnvGuard` restores prior values on drop even on panic.
    unsafe {
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("OVERDRIVE_CONFIG_DIR");
    }

    let output =
        overdrive_cli::commands::cluster::init(InitArgs { config_dir: None, force: false })
            .await
            .expect("cluster::init on HOME fallback");

    let canonical = tmp.path().join(".overdrive").join("config");
    let doubled = tmp.path().join(".overdrive").join(".overdrive").join("config");

    assert!(
        canonical.exists(),
        "cluster::init must write trust triple at canonical $HOME/.overdrive/config; not found at {}",
        canonical.display(),
    );
    assert!(
        !doubled.exists(),
        "regression guard: doubled .overdrive/.overdrive/config must NOT be created; found at {}",
        doubled.display(),
    );
    assert_eq!(
        output.config_path, canonical,
        "InitOutput::config_path must equal the canonical $HOME/.overdrive/config",
    );
}

#[tokio::test]
#[serial_test::serial(env)]
async fn default_config_path_matches_init_write_location_on_home_fallback() {
    let tmp = TempDir::new().expect("tempdir");
    let _env = EnvGuard::capture();

    // SAFETY: `#[serial(env)]` prevents concurrent env mutation; the
    // `EnvGuard` restores prior values on drop even on panic.
    unsafe {
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("OVERDRIVE_CONFIG_DIR");
    }

    // The shared helper (Fix 3) — read-side computation. This is the
    // path `main.rs::default_config_path` delegates to, so whatever
    // this function returns must be exactly where `cluster::init`
    // writes on the HOME fallback. The invariant this test pins is
    // the one the bug violated: read and write sites computing the
    // same canonical path.
    let read_path = overdrive_cli::commands::cluster::default_operator_config_path();

    let output =
        overdrive_cli::commands::cluster::init(InitArgs { config_dir: None, force: false })
            .await
            .expect("cluster::init on HOME fallback");

    assert_eq!(
        read_path, output.config_path,
        "default_operator_config_path() must equal InitOutput::config_path on HOME fallback — read and write sites must compute the same canonical path",
    );
    assert!(
        read_path.exists(),
        "path returned by default_operator_config_path() must be what cluster::init actually wrote",
    );
}

// -------------------------------------------------------------------
// (f) bind failure on occupied port returns CliError
// -------------------------------------------------------------------

#[tokio::test]
async fn serve_run_bind_failure_returns_cli_error() {
    // Occupy a port by spawning a bare tokio TcpListener. Then ask
    // `serve::run` to bind the SAME port — it must fail with a
    // CliError variant carrying an actionable message.
    let occupier = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind occupier");
    let occupied_addr = occupier.local_addr().expect("occupier addr");

    let tmp = TempDir::new().expect("tempdir");
    let args = ServeArgs { bind: occupied_addr, data_dir: tmp.path().to_path_buf() };
    let err = overdrive_cli::commands::serve::run(args)
        .await
        .expect_err("serve::run must fail to bind an already-occupied port");

    // Whatever the exact variant, the rendered message must reference
    // the occupied address so the operator can act on it. The concrete
    // variant is implementation detail (could be CliError::Transport
    // or a dedicated BindFailed), but the Display MUST name the port.
    let rendered = format!("{err}");
    assert!(
        rendered.contains(&occupied_addr.port().to_string()),
        "bind-failure Display must name the offending port; got: {rendered}",
    );

    // Keep `occupier` alive until after the assertion so the port
    // stays held for the duration of the bind attempt.
    drop(occupier);
}
