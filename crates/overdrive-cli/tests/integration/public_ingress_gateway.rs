//! In-process production-composition coverage for Route/status driving ports.
//!
//! State machine: Disabled rejects POST/DELETE before parsing. Enabled Empty
//! accepts one Route; Occupied replays/replaces the same ID and rejects another;
//! withdrawal persists Empty and repeating withdrawal is Unchanged. Status is a
//! separate redacted read projection and read failures map to 500.

#![allow(clippy::doc_markdown, reason = "exact Contract Shape test metadata")]

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use overdrive_cli::commands::deploy::{DeployArgs, DeployResourceOutput, deploy_resource};
use overdrive_cli::http_client::ApiClient;
use overdrive_control_plane::api::SubmitWorkloadRequest;
use overdrive_control_plane::tls_bootstrap::{TrustTriple, load_trust_triple};
use overdrive_control_plane::{
    ServerConfig, ServerHandle, run_server, run_server_with_obs_and_driver,
};
use overdrive_core::aggregate::{DriverInput, JobSpecInput, ResourcesInput, VmInput};
use overdrive_core::api::describe::DescribeSpecOutput;
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::public_ingress::{
    PublicCertifiedKeyId, PublicRouteInput, RouteApplyOutcome, ServiceListenerReferenceInput,
};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::{ObservationStore, ObservationStoreError};
use overdrive_gateway::{GatewayConfig, ManualCertifiedKeyConfig};
use overdrive_host::RealCgroupFs;

struct RunningGateway {
    root: tempfile::TempDir,
    handle: ServerHandle,
    client: reqwest::Client,
    api: ApiClient,
    endpoint: String,
    chain_path: PathBuf,
    key_path: PathBuf,
    key_marker: String,
}

async fn spawn_gateway(enabled: bool) -> RunningGateway {
    spawn_gateway_with_observation(enabled, None).await
}

async fn spawn_gateway_with_observation(
    enabled: bool,
    observation: Option<Arc<overdrive_sim::adapters::observation_store::SimObservationStore>>,
) -> RunningGateway {
    let root = tempfile::tempdir().expect("isolated production composition");
    let data_dir = root.path().join("data");
    let config_dir = root.path().join("config");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("public key");
    let cert = rcgen::CertificateParams::new(vec!["api.example.com".to_owned()])
        .expect("public certificate params")
        .self_signed(&key)
        .expect("public certificate");
    let chain_path = root.path().join("public-chain.pem");
    let key_path = root.path().join("public-key.pem");
    std::fs::write(&chain_path, cert.pem()).expect("public chain");
    let key_pem = key.serialize_pem();
    let key_marker = key_pem
        .lines()
        .find(|line| !line.starts_with("---") && line.len() >= 16)
        .expect("distinctive key payload")[..16]
        .to_owned();
    std::fs::write(&key_path, &key_pem).expect("public key");
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
        .expect("private-key mode");
    assert_eq!(
        std::fs::metadata(&key_path).expect("key metadata").permissions().mode() & 0o777,
        0o600,
        "GatewayConfig fixture requires exact private-key mode",
    );
    let gateway = enabled.then(|| {
        GatewayConfig::new(
            std::net::Ipv4Addr::LOCALHOST,
            ManualCertifiedKeyConfig::new(
                PublicCertifiedKeyId::new("api-origin").expect("key ID"),
                chain_path.clone(),
                key_path.clone(),
            ),
        )
    });
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("operator address"),
        data_dir,
        operator_config_dir: config_dir.clone(),
        gateway,
        dataplane_override: Some(Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new())),
        ..ServerConfig::new(Arc::new(overdrive_sim::adapters::SimKek::for_boot()))
    };
    let handle = if let Some(observation) = observation {
        let obs: Arc<dyn ObservationStore> = observation;
        let driver: Arc<dyn Driver> =
            Arc::new(overdrive_sim::adapters::driver::SimDriver::new(DriverType::Vm));
        run_server_with_obs_and_driver(config, obs, driver)
            .await
            .expect("run_server_with_obs_and_driver")
    } else {
        run_server(config, Arc::new(RealCgroupFs::new())).await.expect("run_server")
    };
    let trust_path = config_dir.join(".overdrive/config");
    let trust = load_trust_triple(&trust_path).expect("operator trust triple");
    let endpoint = trust.endpoint().to_owned();
    let client = operator_client(&trust);
    let api = ApiClient::from_config(&trust_path).expect("production ApiClient");
    RunningGateway { root, handle, client, api, endpoint, chain_path, key_path, key_marker }
}

fn operator_client(trust: &TrustTriple) -> reqwest::Client {
    let ca = reqwest::Certificate::from_pem(trust.ca_cert_pem()).expect("operator CA");
    let mut identity = Vec::from(trust.client_cert_pem());
    identity.extend_from_slice(trust.client_key_pem());
    reqwest::Client::builder()
        .add_root_certificate(ca)
        .identity(reqwest::Identity::from_pem(&identity).expect("operator identity"))
        .build()
        .expect("operator-mTLS client")
}

fn route(id: &str, path: &str) -> PublicRouteInput {
    PublicRouteInput {
        id: id.to_owned(),
        hostname: "api.example.com".to_owned(),
        path: path.to_owned(),
        path_match: "segment_prefix".to_owned(),
        certified_key: "api-origin".to_owned(),
        target: ServiceListenerReferenceInput {
            service: "api".to_owned(),
            port: 8080,
            protocol: "tcp".to_owned(),
        },
    }
}

async fn deploy_referenced_service_and_wait_for_frontend(running: &RunningGateway) {
    let accepted = running
        .api
        .submit_workload(SubmitWorkloadRequest {
            spec: SubmitSpecInput::Service(ServiceSpecInput {
                id: "api".to_owned(),
                replicas: 1,
                resources: ResourcesInput { cpu_milli: 10, memory_bytes: 16 * 1024 * 1024 },
                driver: DriverInput::Vm(VmInput {
                    command: "/bin/sleep".to_owned(),
                    args: vec!["30".to_owned()],
                    kernel: "/kernel".to_owned(),
                    rootfs: "/rootfs".to_owned(),
                }),
                listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
                startup_probes: vec![],
                readiness_probes: vec![],
                liveness_probes: vec![],
            }),
        })
        .await
        .expect("production Service POST");
    assert_eq!(accepted.workload_id, "api", "handler acceptance is not convergence");

    let mut allocation_running = false;
    for _ in 0..200 {
        let snapshot = running
            .api
            .alloc_status_for_workload("api")
            .await
            .expect("production allocation-status read");
        if snapshot.replicas_running == 1
            && snapshot.rows.iter().any(|row| {
                matches!(row.state, overdrive_control_plane::api::AllocStateWire::Running)
            })
        {
            allocation_running = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        allocation_running,
        "referenced Service allocation did not reach Running within the bound",
    );

    let description =
        running.api.describe_workload("api").await.expect("production WorkloadDescription read");
    let DescribeSpecOutput::Service(service) = description.spec else {
        panic!("the referenced workload must remain a Service");
    };
    assert_eq!(service.id, "api");
    assert!(service.vip.try_as_ipv4().is_some(), "Service description must expose its IPv4 VIP");
    assert!(
        service.listeners.iter().any(|listener| {
            listener.port == 8080 && listener.protocol.eq_ignore_ascii_case("tcp")
        }),
        "Service description must expose the declared 8080/tcp listener",
    );
}

async fn wait_for_application_current(running: &RunningGateway) -> serde_json::Value {
    for _ in 0..200 {
        let response = running
            .client
            .get(format!("{}/v1/gateway/status", running.endpoint))
            .send()
            .await
            .expect("gateway status poll");
        if response.status().is_success() {
            let value: serde_json::Value = response.json().await.expect("gateway status JSON");
            if value["application"]["current"].is_object() {
                return value;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("Route handler accepted but Gateway Application did not converge within the bound");
}

/// CONTRACT_SHAPE: bounded-change.
/// Mutation universe: the singleton Public Route Set and one Route response;
/// the predeployed Service, workload stream machinery and every non-Route
/// intent/status row are the preserved complement.
#[tokio::test]
#[ignore = "pending DELIVER production Route/status composition"]
async fn route_post_delete_and_status_use_real_handlers_api_client_and_server() {
    let running = spawn_gateway(true).await;
    deploy_referenced_service_and_wait_for_frontend(&running).await;
    let declared = running.api.submit_route(route("public-api", "/")).await.expect("declare");
    assert_eq!(declared.outcome, RouteApplyOutcome::Declared);
    let replay = running.api.submit_route(route("public-api", "/")).await.expect("replay");
    assert_eq!(replay.outcome, RouteApplyOutcome::Unchanged);
    let replaced = running.api.submit_route(route("public-api", "/v2")).await.expect("replace");
    assert_eq!(replaced.outcome, RouteApplyOutcome::Replaced);

    let conflict = running
        .client
        .post(format!("{}/v1/routes", running.endpoint))
        .json(&route("other-api", "/"))
        .send()
        .await
        .expect("conflict response");
    assert_eq!(conflict.status(), reqwest::StatusCode::CONFLICT);
    let invalid = running
        .client
        .post(format!("{}/v1/routes", running.endpoint))
        .json(&route("public-api", "relative"))
        .send()
        .await
        .expect("validation response");
    assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST);

    let value = wait_for_application_current(&running).await;
    let status_json = serde_json::to_string(&value).expect("real JSON");
    assert_eq!(value["enabled"], true);
    assert_eq!(value["listener_bound"], true);
    assert!(value["application"]["current"].is_object());
    assert_eq!(value["application"]["connect_path"], "ready");
    assert_eq!(value["application"]["gateway_identity"]["state"], "current");
    let gateway_spiffe = value["application"]["gateway_identity"]["spiffe_id"]
        .as_str()
        .expect("populated gateway identity");
    assert!(gateway_spiffe.starts_with("spiffe://overdrive.local/gateway/"));
    assert!(!gateway_spiffe.contains("/alloc/"));
    assert_eq!(value["certified_key"]["state"]["state"], "usable");
    assert!(
        value["hydration"].as_array().is_some_and(|rows| !rows.is_empty()),
        "status must project the referenced Service frontend hydration",
    );
    let forbidden = [
        running.chain_path.to_string_lossy().into_owned(),
        running.key_path.to_string_lossy().into_owned(),
        "PRIVATE KEY".to_owned(),
        "certificate_chain_der".to_owned(),
        "ciphertext".to_owned(),
        "nonce".to_owned(),
        "salt".to_owned(),
        running.key_marker.clone(),
    ];
    for forbidden in forbidden {
        assert!(!status_json.contains(&forbidden), "gateway status leaked {forbidden}");
    }

    let (store_status, store_body) = overdrive_control_plane::error::to_response(
        overdrive_control_plane::error::ControlPlaneError::Intent(
            overdrive_core::traits::intent_store::IntentStoreError::Busy,
        ),
    );
    assert_eq!(store_status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(store_body.error, "internal");

    for expected in ["withdrawn", "unchanged"] {
        let response = running
            .client
            .delete(format!("{}/v1/routes/public-api", running.endpoint))
            .send()
            .await
            .expect("Route DELETE");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body: serde_json::Value = response.json().await.expect("withdraw body");
        assert_eq!(body["outcome"], expected);
    }
    running.handle.shutdown(Duration::from_secs(5)).await.expect("clean shutdown");
}

/// CONTRACT_SHAPE: bounded-change.
/// Mutation universe: no gateway Route/application/custody/identity state; the
/// one subsequently submitted control Job is the only intended mutation and
/// proves the pre-existing workload surface remains live.
#[tokio::test]
#[ignore = "pending DELIVER disabled production composition"]
async fn disabled_gateway_rejects_invalid_route_before_parse_and_preserves_existing_serve() {
    let running = spawn_gateway(false).await;
    let response = running
        .client
        .post(format!("{}/v1/routes", running.endpoint))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(b"{not-json".to_vec())
        .send()
        .await
        .expect("disabled response");
    assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
    let body: serde_json::Value = response.json().await.expect("typed ErrorBody");
    assert_eq!(body["error"], "gateway_disabled");

    let status = running
        .client
        .get(format!("{}/v1/gateway/status", running.endpoint))
        .send()
        .await
        .expect("disabled status");
    assert_eq!(status.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = status.json().await.expect("disabled status JSON");
    assert_eq!(body["enabled"], false);
    assert_eq!(body["listener_bound"], false);
    assert!(body["application"].is_null());
    assert!(body["certified_key"].is_null());
    let workload = running
        .api
        .submit_workload(SubmitWorkloadRequest {
            spec: SubmitSpecInput::Job(JobSpecInput {
                id: "disabled-gateway-control".to_owned(),
                replicas: 1,
                resources: ResourcesInput { cpu_milli: 10, memory_bytes: 16 * 1024 * 1024 },
                driver: DriverInput::Vm(VmInput {
                    command: "/bin/true".to_owned(),
                    args: vec![],
                    kernel: "/kernel".to_owned(),
                    rootfs: "/rootfs".to_owned(),
                }),
            }),
        })
        .await
        .expect("existing workload POST remains live");
    assert_eq!(workload.workload_id, "disabled-gateway-control");
    running.handle.shutdown(Duration::from_secs(5)).await.expect("existing server survives");
}

/// CONTRACT_SHAPE: bounded-change.
/// Mutation universe: the singleton Public Route Set plus the two one-shot
/// Route deploy responses; `stream_workloads`/detach changes neither that
/// universe nor the predeployed Service and opens no workload NDJSON stream.
#[tokio::test]
#[ignore = "pending DELIVER Route deploy dispatch"]
async fn route_detach_and_non_detach_are_identical_one_shot_calls_without_ndjson() {
    let running = spawn_gateway(true).await;
    deploy_referenced_service_and_wait_for_frontend(&running).await;
    let spec = running.root.path().join("route.toml");
    std::fs::write(
        &spec,
        r#"[route]
id = "public-api"
hostname = "api.example.com"
path = "/"
path_match = "segment_prefix"
certified_key = "api-origin"

[route.target]
service = "api"
port = 8080
protocol = "tcp"
"#,
    )
    .expect("Route spec");
    let config_path = running.root.path().join("config/.overdrive/config");
    let detached =
        deploy_resource(DeployArgs { spec: spec.clone(), config_path: config_path.clone() }, false)
            .await
            .expect("detached-shaped Route deploy");
    let non_detached = deploy_resource(DeployArgs { spec, config_path }, true)
        .await
        .expect("TTY-shaped Route deploy");
    let (DeployResourceOutput::Route(detached), DeployResourceOutput::Route(non_detached)) =
        (detached, non_detached)
    else {
        panic!("Route deploy must never enter either workload/NDJSON output variant");
    };
    assert_eq!(detached.route_id, non_detached.route_id);
    assert_eq!(detached.route_generation, non_detached.route_generation);
    assert_eq!(detached.endpoint, non_detached.endpoint);
    assert_eq!(detached.outcome, RouteApplyOutcome::Declared);
    assert_eq!(non_detached.outcome, RouteApplyOutcome::Unchanged);
    running.handle.shutdown(Duration::from_secs(5)).await.expect("clean shutdown");
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER gateway status ObservationStore point reads"]
async fn gateway_status_point_read_failure_maps_to_http_500_without_secret_detail() {
    let observation =
        Arc::new(overdrive_sim::adapters::observation_store::SimObservationStore::single_peer(
            overdrive_core::id::NodeId::new("gateway-node").expect("node ID"),
            991,
        ));
    let running = spawn_gateway_with_observation(true, Some(Arc::clone(&observation))).await;
    deploy_referenced_service_and_wait_for_frontend(&running).await;
    observation.inject_gateway_status_read_failure(ObservationStoreError::Unreachable {
        peer: "status-peer".to_owned(),
    });
    let response = running
        .client
        .get(format!("{}/v1/gateway/status", running.endpoint))
        .send()
        .await
        .expect("status failure response");
    assert_eq!(response.status(), reqwest::StatusCode::INTERNAL_SERVER_ERROR);
    let raw = response.text().await.expect("typed ErrorBody");
    let body: serde_json::Value = serde_json::from_str(&raw).expect("JSON error body");
    assert_eq!(body["error"], "internal");
    for forbidden in ["PRIVATE KEY", "ciphertext", "nonce", "salt"] {
        assert!(!raw.contains(forbidden));
    }
    running.handle.shutdown(Duration::from_secs(5)).await.expect("clean shutdown");
}
