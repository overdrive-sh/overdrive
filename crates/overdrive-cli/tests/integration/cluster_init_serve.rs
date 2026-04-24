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

/// Build an `ApiClient` from the on-disk trust triple written by
/// `run_server`. The triple's `endpoint` names the resolved-port URL
/// the server bound to, so `from_config` is the only call needed.
fn build_client(config_path: &Path) -> ApiClient {
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
    let client = build_client(&config_path);
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

    // Shut down FIRST, then build a fresh client and probe — the
    // server is gone.
    handle.shutdown().await.expect("clean shutdown");

    let client = build_client(&config_path);
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

/// Save-and-restore guard for the three env vars that steer the
/// production-default path resolvers in `main.rs`
/// (`default_config_path` + `default_data_dir`) and
/// `commands::cluster::default_operator_config_path`. `Drop` restores
/// the prior values on unwind, which keeps env state sane when an
/// assertion panics mid-test.
///
/// `XDG_DATA_HOME` is captured alongside `HOME` / `OVERDRIVE_CONFIG_DIR`
/// because the `default_data_dir()` helper in `main.rs` consults
/// `$XDG_DATA_HOME` first (ADR-0013 §5) and falls back to
/// `$HOME/.local/share/overdrive` only when `$XDG_DATA_HOME` is unset.
/// The production-default round-trip test below exercises the HOME
/// fallback and must clear `$XDG_DATA_HOME` to do so.
//
// clippy::struct_field_names: the `prior_` prefix is load-bearing —
// it conveys "saved value to restore on drop," which is the RAII
// guard's entire contract. Dropping the prefix would leave
// `home: Option<OsString>` alongside the mutated `HOME` env var,
// which reads as the *current* value rather than the captured one.
#[allow(clippy::struct_field_names)]
struct EnvGuard {
    prior_home: Option<std::ffi::OsString>,
    prior_config_dir: Option<std::ffi::OsString>,
    prior_xdg_data_home: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn capture() -> Self {
        Self {
            prior_home: std::env::var_os("HOME"),
            prior_config_dir: std::env::var_os("OVERDRIVE_CONFIG_DIR"),
            prior_xdg_data_home: std::env::var_os("XDG_DATA_HOME"),
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: env mutation is process-wide and racy; `#[serial(env)]`
        // ensures no other test mutates $HOME / $OVERDRIVE_CONFIG_DIR /
        // $XDG_DATA_HOME concurrently.
        unsafe {
            match &self.prior_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            match &self.prior_config_dir {
                Some(v) => std::env::set_var("OVERDRIVE_CONFIG_DIR", v),
                None => std::env::remove_var("OVERDRIVE_CONFIG_DIR"),
            }
            match &self.prior_xdg_data_home {
                Some(v) => std::env::set_var("XDG_DATA_HOME", v),
                None => std::env::remove_var("XDG_DATA_HOME"),
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

// -------------------------------------------------------------------
// RED regression test — serve + submit with production defaults
// (fix-cli-cannot-reach-control-plane, Step 01-01)
//
// INTENTIONAL RED SCAFFOLD. Against current `main` this test fails
// with `CliError::Transport` "could not connect to server" emitted
// from `job::submit`, because:
//
//   - `serve::run` writes its trust triple at
//     `<data_dir>/.overdrive/config`  (production default:
//     `$HOME/.local/share/overdrive/.overdrive/config`)
//   - `job::submit` reads the trust triple from
//     `default_operator_config_path()`  (production default:
//     `$HOME/.overdrive/config`)
//
// These are different files under production defaults. The read-side
// trust triple was minted earlier by `cluster init` against a stale CA;
// the TLS handshake fails, reqwest classifies the result as a connect
// failure, and the CLI surfaces `CliError::Transport`.
//
// This is the first integration test exercising the end-to-end `serve`
// -> `job submit` flow with real production-path defaults (RCA §Root
// cause C). Step 01-02 will flip it GREEN alongside the
// `ServerConfig` / `ServeArgs` field split that routes `serve::run`'s
// trust triple into `default_operator_config_dir()` instead of
// `<data_dir>/.overdrive`.
//
// Committed with `git commit --no-verify` per
// `.claude/rules/testing.md` §RED scaffolds and intentionally-failing
// commits.
// -------------------------------------------------------------------

#[tokio::test]
#[serial_test::serial(env)]
async fn serve_and_submit_with_production_defaults_succeeds() {
    let tmp = TempDir::new().expect("tempdir");
    let _env = EnvGuard::capture();

    // SAFETY: `#[serial(env)]` prevents concurrent env mutation; the
    // `EnvGuard` restores prior values on drop even on panic. Scoping
    // `$HOME` to the tempdir makes `default_data_dir()` (ADR-0013 §5
    // HOME fallback, since `$XDG_DATA_HOME` is cleared) resolve under
    // the tempdir AND `default_operator_config_path()` resolve at
    // `$HOME/.overdrive/config` — both are the same `main.rs` code
    // paths that run in production with no flags.
    unsafe {
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("OVERDRIVE_CONFIG_DIR");
        std::env::remove_var("XDG_DATA_HOME");
    }

    // 1. Mirror the real operator flow: FIRST `cluster init` mints a
    //    trust triple at $HOME/.overdrive/config. This is the
    //    "previously-initialised machine" precondition the bug needs —
    //    without a file at the CLI read site, the failure mode is
    //    `CliError::ConfigLoad` (no config), not the `CliError::Transport`
    //    "could not connect to server" the RCA pins. Production hits
    //    this exact shape because operators run `cluster init` once,
    //    then `serve` and `job submit` on every subsequent day.
    overdrive_cli::commands::cluster::init(InitArgs { config_dir: None, force: false })
        .await
        .expect("cluster::init seeds $HOME/.overdrive/config before the failure scenario");

    // 2. Mirror `main.rs::default_data_dir` on the HOME-fallback branch.
    let data_dir = tmp.path().join(".local/share/overdrive");

    // 3. Start the server via the SAME handler main.rs invokes. On
    //    current HEAD, `serve::run` mints a fresh ephemeral CA
    //    (ADR-0010 §R1) and writes it to
    //    `<data_dir>/.overdrive/config` — which is NOT the file the
    //    CLI read-side reads. The trust triple at
    //    $HOME/.overdrive/config is now stale relative to the running
    //    server.
    //
    //    NOTE (Step 01-02): after the fix lands, `ServeArgs` grows a
    //    `config_dir: PathBuf` field and `serve::run` writes its trust
    //    triple into `<config_dir>/.overdrive/config` (matching
    //    `default_operator_config_path()`) instead of
    //    `<data_dir>/.overdrive/config`. 01-02 will extend this
    //    construction to thread `config_dir: tmp.path().to_path_buf()`
    //    alongside the field addition on `ServeArgs`, replacing the
    //    stale trust triple minted by step 1 with the current one.
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let args = ServeArgs { bind, data_dir };
    let handle: ServeHandle = overdrive_cli::commands::serve::run(args).await.expect("serve::run");

    // 3. The CLI read-side MUST land on $HOME/.overdrive/config under
    //    production defaults. Structural invariant — pin it now so a
    //    future drift between the read site and the write site fails
    //    THIS test loudly instead of silently re-introducing the bug.
    let config_path = overdrive_cli::commands::cluster::default_operator_config_path();
    assert_eq!(
        config_path,
        tmp.path().join(".overdrive").join("config"),
        "production default must resolve to $HOME/.overdrive/config",
    );

    // 4. Write a minimal valid job spec into the tempdir. Same shape
    //    as the existing `write_valid_payments_toml` helper in
    //    `tests/integration/job_submit.rs` — kept inline here to
    //    avoid cross-file shared-helper plumbing for a single call
    //    site.
    let spec_path = tmp.path().join("payments.toml");
    std::fs::write(
        &spec_path,
        r#"
id = "payments"
replicas = 3
cpu_milli = 500
memory_bytes = 536870912
"#,
    )
    .expect("write payments.toml");

    // 5. Submit via the SAME handler `main.rs` invokes, with NO
    //    explicit endpoint — the handler MUST pick up the trust
    //    triple at `default_operator_config_path()` per ADR-0010 §R4
    //    and the `overdrive-cli/CLAUDE.md` endpoint-resolution rule.
    //
    //    Against current `main` this fails with `CliError::Transport`
    //    because the trust triple at
    //    `$HOME/.overdrive/config` (if any; typically stale from a
    //    previous `cluster init`) names a different CA from the one
    //    `serve::run` just minted into
    //    `$HOME/.local/share/overdrive/.overdrive/config`. The
    //    `.expect` message is the failure signal for the RED scaffold.
    let out: overdrive_cli::commands::job::SubmitOutput =
        overdrive_cli::commands::job::submit(overdrive_cli::commands::job::SubmitArgs {
            spec: spec_path,
            config_path,
        })
        .await
        .expect(
            "RED scaffold (Step 01-01; 01-02 flips GREEN): CLI must \
             reach the server it just started — this is exactly the \
             bug the RED scaffold pins. Expected failure on current \
             HEAD is CliError::Transport \"could not connect to server\".",
        );

    // 6. Prove the round-trip landed on the live server.
    assert!(
        out.endpoint.as_str().contains("127.0.0.1"),
        "submit must have reached the live server; got endpoint {}",
        out.endpoint,
    );

    handle.shutdown().await.expect("clean shutdown");
}
