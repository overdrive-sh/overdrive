//! Public-ingress gateway — the operator CLI against the production handlers,
//! in-process (S-PIG-36).
//!
//! Every test boots `serve` through the one crate fixture
//! ([`super::serve_fixture`]): `SimDataplaneComposer` over a `SimDataplane`,
//! the real stores, the real operator listener. No subprocess is spawned; the
//! CLI handler (`deploy_resource`) is called directly.
//!
//! **Authored here.** The gateway-disabled composition (`ServerConfig.gateway
//! == None`, which is what the `serve` fixture builds): a Route deploy is
//! rejected with the typed disabled error before the spec is validated and
//! without an intent write (S-PIG-36); and, behind an *enabled* composition
//! (the `--gateway-*` arguments complete, the gateway driven ports the Sim
//! pair, custody over a KEK the custody codec resolves), S-PIG-34 (Route
//! deploy is one-shot for both stream modes), S-PIG-35 (declare, conflict,
//! invalid id, withdraw and the status read) and S-PIG-42 (a status point-read
//! failure is a redacted 500, then a healthy read recovers).
//!
//! The enabled bodies enter through the same production `run_with_adapters`
//! and call `deploy_resource` and `ApiClient::submit_route`. The operator
//! client has no `DELETE /v1/routes/{id}` or `GET /v1/gateway/status` method
//! (see the batch gaps file), so those two requests are issued with a plain
//! HTTPS client built from the trust triple the server wrote.
//!
//! # Bind-address table (R-6)
//!
//! This file owns second octets 170..=179 for the enabled compositions.
//!
//! | `T` | test |
//! |---|---|
//! | 170 | S-PIG-34 (stream mode off) |
//! | 171 | S-PIG-34 (stream mode on) |
//! | 172 | S-PIG-35 |
//! | 173 | S-PIG-42 |

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE metadata tags")]

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine as _;

use overdrive_cli::commands::deploy::{DeployArgs, DeployResourceOutput, deploy_resource};
use overdrive_cli::commands::serve::{self, run_with_adapters};
use overdrive_cli::http_client::{ApiClient, CliError};
use overdrive_control_plane::api::{GatewayStatusResponse, WithdrawRouteResponse};
use overdrive_control_plane::{GatewayAdapters, NodeAdapters};
use overdrive_core::ca::kek::KEK_LEN;
use overdrive_core::ca::root_key_envelope::KekId;
use overdrive_core::id::PublicCertifiedKeyId;
use overdrive_core::public_ingress::{
    PublicRouteInput, RouteApplyOutcome, RouteWithdrawOutcome, ServiceListenerReferenceInput,
};
use overdrive_host::RealCgroupFs;
use overdrive_host::ca::PublicCertifiedKeyAeadCodec;
use overdrive_sim::adapters::SimDataplaneComposer;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::gateway::SimGatewayAdapters;
use overdrive_sim::adapters::kek::SimKek;
use overdrive_store_local::{IntentStore, LocalIntentStore};
use tempfile::TempDir;

/// A grammar-valid Route document.
const VALID_ROUTE: &str = r#"
[route]
id = "public-api"
hostname = "api.example.test"
path = "/"
path_match = "segment_prefix"
certified_key = "api-origin"

[route.target]
service = "api"
port = 8080
protocol = "tcp"
"#;

/// A Route document the server's Route validation refuses (the hostname is not
/// a valid public hostname). With the gateway enabled this is a 400 from
/// validation; with it disabled it must never reach validation.
const UNPARSABLE_ROUTE: &str = r#"
[route]
id = "public-api"
hostname = "NOT A HOST NAME"
path = "relative-path"
path_match = "segment_prefix"
certified_key = "api-origin"

[route.target]
service = "api"
port = 8080
protocol = "tcp"
"#;

async fn spawn_gateway_disabled_server() -> (serve::ServeHandle, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let args = super::serve_fixture::serve_args(bind, data_dir, config_dir);
    let handle = super::serve_fixture::serve(args, super::serve_fixture::DataplaneChoice::Sim)
        .await
        .expect("serve with the gateway disabled");
    (handle, tmp)
}

fn config_path(tmp: &Path) -> PathBuf {
    tmp.join("conf").join(".overdrive").join("config")
}

fn write_spec(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write route spec");
    path
}

/// Every key of the intent store under `data_dir`, read after the server that
/// owned it has shut down.
async fn intent_keys(data_dir: &Path) -> Vec<String> {
    let store =
        LocalIntentStore::open(data_dir.join("intent.redb")).expect("open committed intent store");
    let snapshot = store.export_snapshot().await.expect("export intent snapshot");
    snapshot
        .entries
        .iter()
        .map(|(key, _)| String::from_utf8_lossy(key.as_ref()).into_owned())
        .collect()
}

/// S-PIG-36 — with the gateway disabled by composition, `overdrive deploy
/// <ROUTE_SPEC>` is rejected with the typed disabled error for a valid Route
/// document and for one the Route validation would refuse, in both stream
/// modes; the refusal comes before the spec is validated, so the unparsable
/// document never reaches a validation error, and the intent store gains no
/// Route key.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER route-operator-surface"]
async fn route_deploy_on_a_gateway_disabled_node_is_rejected_before_the_spec_is_parsed() {
    let (handle, tmp) = spawn_gateway_disabled_server().await;
    let config = config_path(tmp.path());

    for (name, body) in
        [("valid-route.toml", VALID_ROUTE), ("unparsable-route.toml", UNPARSABLE_ROUTE)]
    {
        let spec = write_spec(tmp.path(), name, body);
        for stream_workloads in [false, true] {
            let result: Result<DeployResourceOutput, CliError> = deploy_resource(
                DeployArgs { spec: spec.clone(), config_path: config.clone() },
                stream_workloads,
            )
            .await;

            match result {
                Err(CliError::HttpStatus { status, body: error }) => {
                    assert_eq!(status, 409, "{name} (stream={stream_workloads}): status");
                    assert_eq!(error.error, "gateway_disabled", "{name}: error code");
                    assert_eq!(
                        error.message, "public ingress gateway is disabled",
                        "{name}: message"
                    );
                    assert_eq!(error.field, None, "{name}: no offending field is named");
                }
                other => panic!(
                    "{name} (stream={stream_workloads}): expected the typed disabled-gateway \
                     rejection (HTTP 409), got {other:?}"
                ),
            }
        }
    }

    handle.shutdown().await.expect("clean shutdown");
    let keys = intent_keys(&tmp.path().join("data")).await;
    let route_keys: Vec<&String> = keys
        .iter()
        .filter(|key| key.to_ascii_lowercase().contains("route") || key.contains("gateway"))
        .collect();
    assert!(
        route_keys.is_empty(),
        "a gateway-disabled node must write no Route or gateway intent, found {route_keys:?}"
    );
}

// ===========================================================================
// The enabled composition behind the operator listener
// ===========================================================================

/// The public Certified Key the operator names with the `--gateway-*` flags:
/// a self-signed ECDSA P-256 leaf for `api.example.com` valid until 2108, and
/// its PKCS#8 key. Operator-supplied input, not material production installs.
const PUBLIC_CHAIN_PEM: &str = "-----BEGIN CERTIFICATE-----
MIIBozCCAUqgAwIBAgIUcJWe3NwUDOSMTSWEHYX0OsMcrVIwCgYIKoZIzj0EAwIw
GjEYMBYGA1UEAwwPYXBpLmV4YW1wbGUuY29tMCAXDTI2MTEwMzIxMTQ1NloYDzIx
MDgxMjIzMjExNDU2WjAaMRgwFgYDVQQDDA9hcGkuZXhhbXBsZS5jb20wWTATBgcq
hkjOPQIBBggqhkjOPQMBBwNCAAQqxeyHoEg9FTRwBy9HyeByLZbhneiVTTukWYSI
AA/tiiAoI+oXQK9vJcrnW2l41dCuXf4V40qfHB9Uf0wgjVi9o2wwajAdBgNVHQ4E
FgQUvDacuGJeAaMRQoKN/yDhrxArdOwwHwYDVR0jBBgwFoAUvDacuGJeAaMRQoKN
/yDhrxArdOwwGgYDVR0RBBMwEYIPYXBpLmV4YW1wbGUuY29tMAwGA1UdEwEB/wQC
MAAwCgYIKoZIzj0EAwIDRwAwRAIgEFZ9Z/ezSRgY6iyiHuLzektgde9INoTHOPO+
9MVfV7sCIBKgCma8MEtOBeoqVjETVZggcDFFUTcRBriEi+wYYNVd
-----END CERTIFICATE-----
";
const PUBLIC_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgelOtw0sTNGEeCOvN
d5rRQwMKHKioSztHuMpss4D2yomhRANCAAQqxeyHoEg9FTRwBy9HyeByLZbhneiV
TTukWYSIAA/tiiAoI+oXQK9vJcrnW2l41dCuXf4V40qfHB9Uf0wgjVi9
-----END PRIVATE KEY-----
";
const CERTIFIED_KEY_ID: &str = "api-origin";

/// [`VALID_ROUTE`] with a changed path: the same Route ID, new fields.
const VALID_ROUTE_NEW_PATH: &str = r#"
[route]
id = "public-api"
hostname = "api.example.test"
path = "/v2"
path_match = "segment_prefix"
certified_key = "api-origin"

[route.target]
service = "api"
port = 8080
protocol = "tcp"
"#;

/// A booted enabled node and the pieces a test reads it through.
struct EnabledNode {
    handle: serve::ServeHandle,
    tmp: TempDir,
    client: reqwest::Client,
    base: String,
    status_fault: Arc<AtomicBool>,
}

/// Boot `serve` through the production `run_with_adapters` with the gateway
/// enabled by the four `--gateway-*` arguments on `127.<octet>.0.1`. The
/// dataplane composer and the gateway relay and client are the Sim pair; the
/// custody codec resolves a hermetic KEK; the observation store is wrapped by
/// the status-read fault store (disarmed until a test arms it).
#[expect(
    clippy::result_large_err,
    reason = "the bundle builder returns the boot's typed `ControlPlaneError` unboxed, as `run_server` does"
)]
async fn spawn_gateway_enabled_server(octet: u8) -> EnabledNode {
    overdrive_testing::forwarder_module::expect_forwarder_module();
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let chain = tmp.path().join("api-origin-chain.pem");
    let key = tmp.path().join("api-origin-key.pem");
    std::fs::write(&chain, PUBLIC_CHAIN_PEM).expect("write the certificate chain");
    std::fs::write(&key, PUBLIC_KEY_PEM).expect("write the private key");
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600))
            .expect("private-key mode");
    }
    let mut args = super::serve_fixture::serve_args(
        "127.0.0.1:0".parse().expect("parse bind addr"),
        data_dir,
        config_dir.clone(),
    );
    args.gateway_address = Some(Ipv4Addr::new(127, octet, 0, 1));
    args.gateway_certified_key_id =
        Some(PublicCertifiedKeyId::new(CERTIFIED_KEY_ID).expect("certified key id"));
    args.gateway_certificate_chain = Some(chain);
    args.gateway_private_key = Some(key);

    let status_fault = Arc::new(AtomicBool::new(false));
    let armed = Arc::clone(&status_fault);
    let handle = run_with_adapters(args, Arc::new(RealCgroupFs::new()), move |config, fs| {
        let mut adapters = NodeAdapters::host(config, fs, Arc::new(SimKek::for_boot()))?;
        adapters.boot.dataplane =
            Arc::new(SimDataplaneComposer::new(Arc::new(SimDataplane::new()), Ipv4Addr::LOCALHOST));
        let sim = SimGatewayAdapters::new(Arc::clone(&adapters.boot.clock));
        adapters.boot.gateway = GatewayAdapters {
            certified_key_codec: Arc::new(PublicCertifiedKeyAeadCodec::new(Arc::new(
                SimKek::new().with(
                    &KekId::new("overdrive-public-certified-key").expect("custody KEK id"),
                    [0x5a; KEK_LEN],
                ),
            ))),
            client_mtls: sim.client_mtls(),
            relay: sim.relay(),
        };
        adapters.boot.observation = Arc::new(super::status_fault_store::StatusReadFaultStore {
            inner: Arc::clone(&adapters.boot.observation),
            armed: Arc::clone(&armed),
        });
        Ok(adapters)
    })
    .await
    .expect("serve with the gateway enabled");
    let client = trusting_client(&config_dir);
    let base = handle.endpoint().as_str().trim_end_matches('/').to_owned();
    EnabledNode { handle, tmp, client, base, status_fault }
}

/// An HTTPS client that trusts the CA the server wrote to the trust triple.
fn trusting_client(config_dir: &Path) -> reqwest::Client {
    let text = std::fs::read_to_string(config_dir.join(".overdrive").join("config"))
        .expect("read the trust triple");
    let doc: toml::Value = toml::from_str(&text).expect("parse the trust triple");
    let ca_b64 = doc
        .get("contexts")
        .and_then(toml::Value::as_array)
        .and_then(|contexts| contexts.first())
        .and_then(|context| context.get("ca"))
        .and_then(toml::Value::as_str)
        .expect("a context with a ca field");
    let ca = base64::engine::general_purpose::STANDARD.decode(ca_b64).expect("base64 ca");
    reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(&ca).expect("parse CA PEM"))
        .https_only(true)
        .use_rustls_tls()
        .build()
        .expect("build reqwest client")
}

fn route_input(id: &str, path: &str) -> PublicRouteInput {
    PublicRouteInput {
        id: id.to_owned(),
        hostname: "api.example.test".to_owned(),
        path: path.to_owned(),
        path_match: "segment_prefix".to_owned(),
        certified_key: CERTIFIED_KEY_ID.to_owned(),
        target: ServiceListenerReferenceInput {
            service: "api".to_owned(),
            port: 8080,
            protocol: "tcp".to_owned(),
        },
    }
}

async fn status_of(node: &EnabledNode) -> (u16, String) {
    let response = node
        .client
        .get(format!("{}/v1/gateway/status", node.base))
        .send()
        .await
        .expect("GET /v1/gateway/status");
    (response.status().as_u16(), response.text().await.expect("status body"))
}

/// S-PIG-34 — `overdrive deploy <ROUTE_SPEC>` is a one-shot POST with no
/// streaming wait, and its outcome is identical for both stream modes: a first
/// deploy of a document is `Declared` in either mode (same Route ID and
/// canonical generation), an identical re-deploy is `Unchanged` in the other
/// mode, a changed document with the same ID is `Replaced`, and the Route Set
/// holds the Route after the server stops.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER route-operator-surface"]
async fn route_deploy_is_one_shot_with_the_same_outcome_in_both_stream_modes() {
    let mut first_outcomes = Vec::new();
    for (stream_workloads, octet) in [(false, 170_u8), (true, 171_u8)] {
        let node = spawn_gateway_enabled_server(octet).await;
        let config = config_path(node.tmp.path());
        let spec = write_spec(node.tmp.path(), "route.toml", VALID_ROUTE);

        let started = Instant::now();
        let declared = deploy_resource(
            DeployArgs { spec: spec.clone(), config_path: config.clone() },
            stream_workloads,
        )
        .await
        .expect("the Route deploy succeeds");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "a Route deploy is one-shot and waits for no stream"
        );
        let DeployResourceOutput::Route(declared) = declared else {
            panic!("a Route document yields the one-shot Route output, got {declared:?}");
        };
        assert_eq!(declared.outcome, RouteApplyOutcome::Declared, "stream={stream_workloads}");
        assert_eq!(declared.route_id, "public-api");
        first_outcomes.push((declared.route_id.clone(), declared.route_generation.clone()));

        // The identical document again, in the opposite mode: Unchanged.
        let again = deploy_resource(
            DeployArgs { spec: spec.clone(), config_path: config.clone() },
            !stream_workloads,
        )
        .await
        .expect("the identical Route deploy succeeds");
        let DeployResourceOutput::Route(again) = again else { panic!("Route output expected") };
        assert_eq!(again.outcome, RouteApplyOutcome::Unchanged, "an identical re-apply");
        assert_eq!(again.route_generation, declared.route_generation);

        // Same ID, new path: Replaced with a new generation.
        let changed = write_spec(node.tmp.path(), "route-v2.toml", VALID_ROUTE_NEW_PATH);
        let replaced = deploy_resource(
            DeployArgs { spec: changed, config_path: config.clone() },
            stream_workloads,
        )
        .await
        .expect("the changed Route deploy succeeds");
        let DeployResourceOutput::Route(replaced) = replaced else {
            panic!("Route output expected")
        };
        assert_eq!(replaced.outcome, RouteApplyOutcome::Replaced);
        assert_ne!(replaced.route_generation, declared.route_generation);

        let data_dir = node.tmp.path().join("data");
        node.handle.shutdown().await.expect("clean shutdown");
        let keys = intent_keys(&data_dir).await;
        assert!(
            keys.iter().any(|key| key.to_ascii_lowercase().contains("route")),
            "the Route Set holds the Route: {keys:?}"
        );
        drop(node.tmp);
    }
    assert_eq!(first_outcomes[0], first_outcomes[1], "both modes declare the same Route");
}

/// S-PIG-35 — through the operator client against the production handlers: a
/// declare is 201 `Declared`; a different Route ID while one is live is a 409
/// kept as the typed `CliError::HttpStatus`; an invalid Route ID is a 400; a
/// withdrawal is 200 `Withdrawn`, a repeat is 200 `Unchanged`; and the status
/// read is the redacted `GatewayStatusResponse` (enabled, configured address),
/// apart from the typed client errors.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER route-operator-surface"]
async fn the_operator_client_declares_conflicts_rejects_withdraws_and_reads_status() {
    let node = spawn_gateway_enabled_server(172).await;
    let api = ApiClient::from_config(&config_path(node.tmp.path())).expect("operator client");

    let declared =
        api.submit_route(route_input("public-api", "/")).await.expect("declare the first Route");
    assert_eq!(declared.outcome, RouteApplyOutcome::Declared);

    match api.submit_route(route_input("other-api", "/")).await {
        Err(CliError::HttpStatus { status, body }) => {
            assert_eq!(status, 409, "a different Route while one is live conflicts");
            assert_eq!(body.error, "conflict");
        }
        other => panic!("expected the typed 409 conflict, got {other:?}"),
    }
    match api.submit_route(route_input("Not A Valid Route Id", "/")).await {
        Err(CliError::HttpStatus { status, body }) => {
            assert_eq!(status, 400, "an invalid Route ID is a validation error");
            assert_eq!(body.error, "validation");
        }
        other => panic!("expected the typed 400 validation error, got {other:?}"),
    }

    let withdrawn: WithdrawRouteResponse = {
        let response = node
            .client
            .delete(format!("{}/v1/routes/public-api", node.base))
            .send()
            .await
            .expect("DELETE /v1/routes/public-api");
        assert_eq!(response.status().as_u16(), 200);
        response.json().await.expect("WithdrawRouteResponse body")
    };
    assert_eq!(withdrawn.route_id, "public-api");
    assert_eq!(withdrawn.outcome, RouteWithdrawOutcome::Withdrawn);
    let repeated: WithdrawRouteResponse = node
        .client
        .delete(format!("{}/v1/routes/public-api", node.base))
        .send()
        .await
        .expect("repeat DELETE")
        .json()
        .await
        .expect("WithdrawRouteResponse body");
    assert_eq!(repeated.outcome, RouteWithdrawOutcome::Unchanged, "the set is already empty");

    let (code, body) = status_of(&node).await;
    assert_eq!(code, 200, "the status read is a plain 200");
    let status: GatewayStatusResponse = serde_json::from_str(&body).expect("status body");
    assert!(status.enabled, "the gateway is enabled");
    assert_eq!(
        status.configured_address.map(|address| address.to_string()),
        Some("127.172.0.1:443".to_owned()),
        "the configured bind is reported"
    );
    node.handle.shutdown().await.expect("clean shutdown");
}

/// S-PIG-42 — a point-read failure behind `GET /v1/gateway/status` is a 500
/// with a fixed redacted body: it carries none of the injected error text, no
/// path, no key or ciphertext text. Once the fault is lifted the next read is
/// the normal 200 projection.
///
/// The fault is a delegating observation store at the existing driven port;
/// the operator client has no status method, so the read is a plain request.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER route-operator-surface"]
async fn a_status_point_read_failure_is_a_redacted_500_and_the_next_read_recovers() {
    let node = spawn_gateway_enabled_server(173).await;
    let (healthy, _) = status_of(&node).await;
    assert_eq!(healthy, 200, "the baseline read is healthy");

    node.status_fault.store(true, Ordering::SeqCst);
    let (code, body) = status_of(&node).await;
    assert_eq!(code, 500, "a failed point read is a server error");
    for forbidden in [
        super::status_fault_store::FAULT_MARKER,
        "api-origin-chain.pem",
        "api-origin-key.pem",
        "PRIVATE KEY",
        "BEGIN CERTIFICATE",
        "ciphertext",
        "intent.redb",
    ] {
        assert!(!body.contains(forbidden), "the 500 body leaks {forbidden:?}: {body}");
    }
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("the 500 body is JSON");
    assert!(
        parsed.get("error").and_then(serde_json::Value::as_str).is_some(),
        "the 500 body is the typed error shape: {body}"
    );

    node.status_fault.store(false, Ordering::SeqCst);
    let (recovered, body) = status_of(&node).await;
    assert_eq!(recovered, 200, "the next healthy read recovers");
    let status: GatewayStatusResponse = serde_json::from_str(&body).expect("status body");
    assert!(status.enabled, "the normal projection is served again");
    node.handle.shutdown().await.expect("clean shutdown");
}
