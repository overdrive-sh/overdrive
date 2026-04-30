//! Integration tests for `overdrive_cli::http_client::ApiClient` — the
//! hand-rolled reqwest client per ADR-0014.
//!
//! These tests are Tier 3 (real reqwest + real TLS + real in-process
//! control-plane server) per `.claude/rules/testing.md`. They stand up
//! the server via `overdrive_control_plane::run_server` on an ephemeral
//! port (NOT a subprocess per `crates/overdrive-cli/CLAUDE.md`), point
//! an `ApiClient` loaded from the on-disk trust triple at the server,
//! and exercise the five endpoint methods plus the failure modes.
//!
//! Acceptance coverage:
//!   (a) `from_config` loads + pins CA + builds client
//!   (b) `cluster_status` against real server returns Ok
//!   (c) `submit_job` + `describe_job` round-trip through HTTP
//!   (d) `cluster_status` with no server → typed `CliError::Transport`
//!       with actionable (non-leaky) Display
//!   (e) `submit_job` with invalid spec → `CliError::HttpStatus { 400,
//!       ErrorBody { error: "validation", field: Some("replicas"), .. } }`
//!   (f) `from_config` with malformed base64 CA → `CliError::ConfigLoad`
//!       with non-raw cause

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use overdrive_cli::http_client::{ApiClient, CliError};
use overdrive_control_plane::api::{IdempotencyOutcome, JobDescription, SubmitJobRequest};
use overdrive_control_plane::tls_bootstrap::{mint_ephemeral_ca, write_trust_triple};
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server};
use overdrive_core::aggregate::{DriverInput, ExecInput, JobSpecInput, ResourcesInput};
use tempfile::TempDir;

/// Spawn a server on an ephemeral port and return (handle, bound addr,
/// tempdir kept alive, path to the trust-triple config). `data_dir`
/// and `operator_config_dir` are SEPARATE subdirectories of the tempdir
/// per `fix-cli-cannot-reach-control-plane` Step 01-02 (RCA §WHY 4C):
/// `data_dir` is the redb storage root; the trust triple is written to
/// `<operator_config_dir>/.overdrive/config`.
async fn spawn_server() -> (ServerHandle, SocketAddr, TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let operator_config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&operator_config_dir).expect("create operator config dir");
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("parse bind addr"),
        data_dir,
        operator_config_dir: operator_config_dir.clone(),
        // CLI integration tests don't start real workloads; bypass
        // the cgroup pre-flight so they run uniformly on macOS and
        // on Linux without delegation.
        allow_no_cgroups: true,
        // `tick_cadence` + `clock` default per
        // `fix-convergence-loop-not-spawned` Step 01-02.
        ..Default::default()
    };
    let handle: ServerHandle = run_server(config).await.expect("run_server");
    let bound: SocketAddr = handle.local_addr().await.expect("bound addr");
    let config_path = operator_config_dir.join(".overdrive").join("config");
    (handle, bound, tmp, config_path)
}

/// Build an `ApiClient` from the on-disk trust triple written by
/// `run_server`. The triple's `endpoint` names the resolved-port URL
/// the server bound to, so `from_config` is the only call needed.
fn build_client_for(config_path: &Path) -> ApiClient {
    ApiClient::from_config(config_path).expect("build ApiClient from trust triple")
}

// -------------------------------------------------------------------
// (a) from_config loads the trust triple and builds a client
// -------------------------------------------------------------------

#[tokio::test]
async fn from_config_loads_trust_triple_and_builds_client() {
    let (handle, _bound, _tmp, config_path) = spawn_server().await;

    // No endpoint override — consume whatever `run_server` wrote.
    let client = ApiClient::from_config(&config_path).expect("build ApiClient from on-disk config");

    // The recorded endpoint should be a parseable URL, non-empty.
    assert!(!client.base_url().as_str().is_empty(), "base URL must not be empty");
    assert_eq!(client.base_url().scheme(), "https", "must be HTTPS per ADR-0010");

    // Prove the client is not merely "constructed" — it can hit the
    // server when pointed at the real bound port. This kills a
    // mutation where `from_config` returns a client that trusts the
    // wrong CA (TLS handshake would fail).
    //
    // Uses `node_list` as the proof-of-life endpoint: `/v1/nodes` is
    // the real observation-read handler wired in step 03-03 and
    // renders `{"rows":[]}` on a fresh store. The `/v1/cluster/info`
    // route is still stubbed until step 03-05 wires the real handler,
    // so we cannot decode it as `ClusterStatus` from this test.
    let live = build_client_for(&config_path);
    let nodes = live.node_list().await.expect("live node_list");
    assert!(nodes.rows.is_empty(), "fresh store must report zero node rows");

    handle.shutdown(Duration::from_secs(2)).await;
}

// -------------------------------------------------------------------
// (b) node_list against in-process server returns Ok
// -------------------------------------------------------------------
//
// Pins the `/v1/nodes` observation-read endpoint that IS wired to the
// real handler as of step 03-03. `/v1/nodes` returns `{"rows":[]}` on
// a fresh store and decodes deterministically into `NodeList`.
//
// `/v1/allocs` was previously exercised via the bare `alloc_status()`
// method; the bare-GET shape is gone (S-AS-09 / single-cut greenfield).
// `?job=<id>` coverage lives in the CLI's `alloc_status_for_job`
// integration tests and the control-plane's `acceptance::alloc_status_snapshot`.

#[tokio::test]
async fn node_list_against_in_process_server_returns_ok() {
    let (handle, _bound, _tmp, config_path) = spawn_server().await;
    let client = build_client_for(&config_path);

    let nodes = client.node_list().await.expect("node_list");
    assert!(nodes.rows.is_empty(), "fresh store must report zero node rows");

    handle.shutdown(Duration::from_secs(2)).await;
}

// -------------------------------------------------------------------
// (c) submit_job + describe_job round-trip
// -------------------------------------------------------------------

#[tokio::test]
async fn submit_job_then_describe_round_trips_via_http_client() {
    let (handle, _bound, _tmp, config_path) = spawn_server().await;
    let client = build_client_for(&config_path);

    let spec = JobSpecInput {
        id: "payments".to_owned(),
        replicas: 3,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 536_870_912 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    };

    let submit_resp =
        client.submit_job(SubmitJobRequest { spec: spec.clone() }).await.expect("submit_job");
    assert!(!submit_resp.job_id.is_empty(), "job_id must not be empty");
    // Per ADR-0020 the per-write witness is `outcome` + `spec_digest`;
    // a fresh insert reports `outcome = Inserted` with a 64-char digest.
    assert_eq!(
        submit_resp.outcome,
        IdempotencyOutcome::Inserted,
        "fresh submit must report `outcome = Inserted`; got {:?}",
        submit_resp.outcome,
    );
    assert_eq!(
        submit_resp.spec_digest.len(),
        64,
        "spec_digest must be 64 hex chars (SHA-256); got {} chars",
        submit_resp.spec_digest.len(),
    );

    let description: JobDescription =
        client.describe_job(&submit_resp.job_id).await.expect("describe_job");
    assert_eq!(description.spec, spec, "round-tripped spec must match submitted spec");
    assert_eq!(
        description.spec_digest, submit_resp.spec_digest,
        "describe must echo the same spec_digest submit returned — \
         the round-trip witness submit and describe agree on the same \
         canonical bytes (ADR-0020).",
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

// -------------------------------------------------------------------
// (d) no server → CliError::Transport with actionable, non-leaky Display
// -------------------------------------------------------------------

#[tokio::test]
async fn cluster_status_with_no_server_returns_transport_error_with_actionable_message() {
    // Spawn a server to write a valid trust triple, then immediately
    // shut it down. `from_config` succeeds (file on disk is valid);
    // the subsequent `cluster_status` call fails because nothing is
    // listening on the chosen port.
    let (handle, bound, _tmp, config_path) = spawn_server().await;
    handle.shutdown(Duration::from_secs(1)).await;

    // Point the client at the now-closed port. `from_config` does NOT
    // attempt to connect — it only loads the trust material.
    let client = build_client_for(&config_path);

    let err = client.cluster_status().await.expect_err("no server → transport error");

    // Variant-level assertion: must be Transport.
    match &err {
        CliError::Transport { endpoint, .. } => {
            assert!(
                endpoint.contains(&bound.port().to_string()),
                "Transport.endpoint must name the endpoint; got {endpoint}",
            );
        }
        other => panic!("expected CliError::Transport, got {other:?}"),
    }

    // Display-level assertion: message must be actionable (names the
    // endpoint) and must NOT leak low-level transport internals.
    let rendered = format!("{err}");
    assert!(
        rendered.contains("127.0.0.1") || rendered.contains("localhost"),
        "Display must name the endpoint so operators can act on the error; got: {rendered}"
    );
    assert!(
        !rendered.contains("ECONNREFUSED"),
        "Display must not leak raw `ECONNREFUSED` token; got: {rendered}",
    );
    assert!(
        !rendered.contains("reqwest::Error"),
        "Display must not leak reqwest::Error Debug format; got: {rendered}",
    );
}

// -------------------------------------------------------------------
// (e) invalid spec → HTTP 400 mapped to CliError::HttpStatus
// -------------------------------------------------------------------

#[tokio::test]
async fn submit_with_invalid_spec_returns_http_status_400_with_error_body() {
    let (handle, _bound, _tmp, config_path) = spawn_server().await;
    let client = build_client_for(&config_path);

    // `replicas = 0` fails `Job::from_spec` at the NonZeroU32 gate;
    // server returns 400 with ErrorBody { error: "validation",
    // field: Some("replicas"), .. } per ADR-0015.
    let bad = JobSpecInput {
        id: "payments".to_owned(),
        replicas: 0,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 536_870_912 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    };

    let err = client.submit_job(SubmitJobRequest { spec: bad }).await.expect_err("bad spec");

    match &err {
        CliError::HttpStatus { status, body } => {
            assert_eq!(*status, 400, "must map HTTP 400 to HttpStatus with status 400");
            assert_eq!(body.error, "validation", "error kind must be 'validation'");
            assert_eq!(
                body.field.as_deref(),
                Some("replicas"),
                "field must name the offending spec field; got {:?}",
                body.field,
            );
            assert!(
                body.message.contains("replica"),
                "message must name the offending field (substring `replica`); got {:?}",
                body.message,
            );
        }
        other => panic!("expected CliError::HttpStatus, got {other:?}"),
    }

    handle.shutdown(Duration::from_secs(2)).await;
}

// -------------------------------------------------------------------
// (f) malformed base64 CA → CliError::ConfigLoad
// -------------------------------------------------------------------

#[tokio::test]
async fn from_config_with_malformed_base64_ca_returns_config_load_error() {
    let tmp = TempDir::new().expect("tempdir");
    let overdrive_dir = tmp.path().join(".overdrive");
    std::fs::create_dir_all(&overdrive_dir).expect("create .overdrive dir");
    let config_path = overdrive_dir.join("config");

    // Write a Talos-shape YAML whose `ca` field is deliberately
    // corrupt base64 — contains invalid characters that no base64
    // decoder will accept.
    let corrupt_yaml = "context: local
contexts:
  local:
    endpoint: https://127.0.0.1:7001
    ca: not-base64!!@#$%
    crt: dmFsaWQgZW5vdWdo
    key: dmFsaWQgZW5vdWdo
";
    std::fs::write(&config_path, corrupt_yaml).expect("write corrupt config");

    let err = ApiClient::from_config(&config_path).expect_err("malformed base64 must fail");

    match &err {
        CliError::ConfigLoad { path, .. } => {
            assert!(
                path.contains(config_path.file_name().unwrap().to_str().unwrap())
                    || path.contains(".overdrive"),
                "ConfigLoad.path must name the config file; got {path}",
            );
        }
        other => panic!("expected CliError::ConfigLoad, got {other:?}"),
    }

    // Display must be actionable (names the path) and not leak decoder
    // Debug format.
    let rendered = format!("{err}");
    assert!(
        rendered.contains(".overdrive") || rendered.contains("config"),
        "Display must name the config path; got: {rendered}",
    );
    assert!(
        !rendered.contains("DecodeError"),
        "Display must not leak base64::DecodeError Debug format; got: {rendered}",
    );
}

// -------------------------------------------------------------------
// (g) TLS handshake failure (mismatched CA) → CliError::Transport
//     with TLS-handshake-specific cause distinct from TCP-refused
//
// Step 01-03 — `fix-cli-cannot-reach-control-plane`. Pins the
// classifier split in `stringify_reqwest_error` so a rustls
// cert-verification error in the `reqwest::Error` source chain renders
// as 'TLS handshake' / 'certificate' rather than collapsing into the
// 'could not connect to server' message reserved for pure
// TCP-`ECONNREFUSED`. Without the split the operator chases a network
// problem when the real fault is trust material.
//
// Test shape: stand up a real in-process server with mint #1, build an
// `ApiClient` from a SECOND independently-minted trust triple whose
// endpoint points at server #1's bound address, then issue any request.
// The TCP handshake completes (server is listening); the TLS handshake
// fails (server presents CA #1, client trusts only CA #2). The
// resulting `CliError::Transport.cause` MUST name TLS / certificate so
// the operator's hint is "re-mint or check trust material", not
// "check the network".
// -------------------------------------------------------------------

#[tokio::test]
async fn stringify_reqwest_error_reports_tls_handshake_distinctly_from_tcp_refused() {
    // Server side: spawn the real control plane on an ephemeral port.
    // `run_server` mints CA #1 and writes the trust triple to
    // `<operator_config_dir_a>/.overdrive/config`. We deliberately do
    // NOT load that triple — it would make the handshake succeed.
    let (handle, bound, _tmp_a, _config_path_a) = spawn_server().await;

    // Client side: mint a SECOND, independent CA. This produces a
    // bytewise-different CA cert (per `tls_bootstrap` test
    // `mint_ephemeral_ca_is_unique_per_call`), so the client's pinned
    // root will NOT verify the server's leaf.
    let tmp_b = TempDir::new().expect("tempdir for mismatched-CA client config");
    let other_material = mint_ephemeral_ca().expect("second independent mint");

    // Write a trust triple under `tmp_b` whose endpoint NAMES SERVER A
    // — but whose CA is from mint #2. `write_trust_triple` writes to
    // `<config_dir>/.overdrive/config` per ADR-0019.
    let target_endpoint = format!("https://{bound}");
    write_trust_triple(tmp_b.path(), &target_endpoint, &other_material)
        .expect("write mismatched-CA trust triple");
    let mismatched_config_path = tmp_b.path().join(".overdrive").join("config");

    // `from_config` succeeds — the file is well-formed; the mismatch
    // only manifests on the wire during the TLS handshake.
    let client = ApiClient::from_config(&mismatched_config_path)
        .expect("from_config: mismatched-CA triple is structurally valid");

    // Any endpoint exercises the same code path; `cluster_status` is
    // shortest. TCP completes (server is listening); TLS handshake
    // fails (cert verification rejects unknown CA).
    let err = client.cluster_status().await.expect_err("mismatched CA → TLS handshake must fail");

    // Variant-level: must surface as Transport (the failure class is
    // still transport, just the cause string is different).
    let rendered = match &err {
        CliError::Transport { endpoint, cause } => {
            assert!(
                endpoint.contains(&bound.port().to_string()),
                "Transport.endpoint must name the live endpoint; got {endpoint}",
            );
            // The Display rendering — operator-facing message — is
            // what `stringify_reqwest_error` ultimately feeds via
            // `cause`. Pin both the variant cause and the rendered
            // form so a future refactor of `Display` cannot drop the
            // distinguishing token.
            format!("{err} | cause={cause}")
        }
        other => panic!("expected CliError::Transport for TLS handshake failure, got {other:?}"),
    };

    // The classifier split: the message must reference 'TLS handshake'
    // or 'certificate' DISTINCT from the 'could not connect to server'
    // string reserved for pure TCP-refused. Either token alone is
    // sufficient — they describe the same root cause from slightly
    // different angles, and rustls' wording shifts across versions.
    let names_tls_or_cert = rendered.contains("TLS handshake") || rendered.contains("certificate");
    assert!(
        names_tls_or_cert,
        "TLS handshake failure must render with 'TLS handshake' or 'certificate' in the \
         message so operators recognise the trust-material fault distinctly from \
         TCP-refused; got: {rendered}",
    );

    // Negative cross-check: the pure-TCP-refused string must NOT
    // appear, otherwise the split has not happened. Without this the
    // assertion above could pass on a message that contains BOTH
    // strings (e.g. an erroneous append).
    assert!(
        !rendered.contains("could not connect to server"),
        "TLS handshake failure must NOT render as 'could not connect to server' — that \
         message is reserved for pure TCP-refused; got: {rendered}",
    );

    // Hint regression: the Display format string in `CliError::Transport`
    // already carries `hint: check that the server is running and the
    // endpoint is correct`, but the TLS-specific cause SHOULD direct
    // the operator at re-running `overdrive serve` (the canonical write
    // site after Step 01-02) since the fault is trust material, not
    // reachability.
    assert!(
        rendered.contains("overdrive serve"),
        "TLS-handshake message must hint at re-running `overdrive serve` since \
         the canonical trust-material write site is `serve` after Step 01-02; \
         got: {rendered}",
    );

    handle.shutdown(Duration::from_secs(2)).await;
}
