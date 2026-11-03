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
//! without an intent write (S-PIG-36).
//!
//! **Not authored here.** S-PIG-34 (Route deploy one-shot for both detach
//! modes), S-PIG-35 (declare, conflict, invalid id, delete, status through
//! `ApiClient`) and S-PIG-42 (status point-read failure) each need an *enabled*
//! gateway composition behind the operator listener; the disabled baseline alone
//! is not those scenarios.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE metadata tags")]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use overdrive_cli::commands::deploy::{DeployArgs, DeployResourceOutput, deploy_resource};
use overdrive_cli::commands::serve;
use overdrive_cli::http_client::CliError;
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
