//! Service VM deploy-lane acceptance tests.

#![allow(
    clippy::doc_markdown,
    reason = "the required per-test CONTRACT_SHAPE declaration is a literal protocol marker"
)]

use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Json, State};
use axum::http::header;
use axum::routing::post;
use axum_server::tls_rustls::RustlsConfig;
use overdrive_cli::commands::deploy::DeployArgs;
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_control_plane::api::{IdempotencyOutcome, SubmitWorkloadRequest};
use overdrive_control_plane::streaming::ServiceSubmitEvent;
use overdrive_control_plane::tls_bootstrap::{mint_ephemeral_ca, write_trust_triple};
use overdrive_core::aggregate::{DriverInput, IntentKey, WorkloadDriver, WorkloadIntent};
use overdrive_core::id::WorkloadId;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::transition_reason::StoppedBy;
use overdrive_store_local::LocalIntentStore;
use serial_test::serial;
use tempfile::TempDir;
use tokio::sync::Mutex;

const VM_SERVICE_TOML: &str = r#"
[service]
id = "vm-service"
replicas = 1

[vm]
command = "/bin/server"
args = ["--serve"]
kernel = "/kernel"
rootfs = "/rootfs"

[resources]
cpu_milli = 100
memory_bytes = 1048576

[[listener]]
port = 8080
protocol = "tcp"
"#;

async fn spawn_server() -> (ServeHandle, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data directory");
    std::fs::create_dir_all(&config_dir).expect("create config directory");

    let handle = overdrive_cli::commands::serve::run_with_dataplane(
        ServeArgs {
            bind: "127.0.0.1:0".parse::<SocketAddr>().expect("socket address"),
            data_dir,
            config_dir,
        },
        std::sync::Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
    )
    .await
    .expect("serve must start");

    (handle, tmp)
}

fn config_path(tmp: &Path) -> PathBuf {
    tmp.join("conf").join(".overdrive").join("config")
}

fn intent_path(tmp: &Path) -> PathBuf {
    tmp.join("data").join("intent.redb")
}

fn write_vm_service(tmp: &Path) -> PathBuf {
    let path = tmp.join("vm-service.toml");
    std::fs::write(&path, VM_SERVICE_TOML).expect("write VM Service TOML");
    path
}

async fn stored_vm_driver(tmp: &Path) -> WorkloadDriver {
    let store = LocalIntentStore::open(intent_path(tmp)).expect("open committed intent store");
    let workload_id = WorkloadId::from_str("vm-service").expect("valid workload id");
    let key = IntentKey::for_workload(&workload_id);
    let bytes = store
        .get(key.as_bytes())
        .await
        .expect("read committed intent")
        .expect("the Service request must commit exactly one intent");
    let intent = WorkloadIntent::from_store_bytes(&bytes, &intent_path(tmp), None)
        .expect("decode committed Service intent");
    let WorkloadIntent::Service(service) = intent else {
        panic!("the Service deploy lanes must submit SubmitSpecInput::Service");
    };
    service.driver
}

fn assert_vm_driver_preserved(driver: WorkloadDriver) {
    let WorkloadDriver::Vm(vm) = driver else {
        panic!("the selected VM driver arm must not be collapsed to Exec");
    };
    assert_eq!(vm.command, "/bin/server");
    assert_eq!(vm.args, ["--serve"]);
    assert_eq!(vm.kernel, "/kernel");
    assert_eq!(vm.rootfs, "/rootfs");
}

fn assert_vm_driver_input_preserved(driver: DriverInput) {
    let DriverInput::Vm(vm) = driver else {
        panic!("the selected VM driver arm must not be collapsed to Exec");
    };
    assert_eq!(vm.command, "/bin/server");
    assert_eq!(vm.args, ["--serve"]);
    assert_eq!(vm.kernel, "/kernel");
    assert_eq!(vm.rootfs, "/rootfs");
}

async fn stream_terminal_response(
    State(request_tx): State<
        Arc<Mutex<Option<tokio::sync::oneshot::Sender<SubmitWorkloadRequest>>>>,
    >,
    Json(request): Json<SubmitWorkloadRequest>,
) -> ([(header::HeaderName, &'static str); 1], String) {
    request_tx
        .lock()
        .await
        .take()
        .expect("the capture server accepts exactly one streaming request")
        .send(request)
        .expect("streaming client must await the captured request");

    let accepted = ServiceSubmitEvent::Accepted {
        spec_digest: "a".repeat(64),
        intent_key: "workloads/vm-service".to_owned(),
        outcome: IdempotencyOutcome::Inserted,
    };
    let stopped = ServiceSubmitEvent::Stopped {
        alloc_id: "alloc-vm-service".to_owned(),
        by: StoppedBy::Operator,
    };
    let response = format!(
        "{}\n{}\n",
        serde_json::to_string(&accepted).expect("Accepted event serialises"),
        serde_json::to_string(&stopped).expect("Stopped event serialises"),
    );

    ([(header::CONTENT_TYPE, "application/x-ndjson")], response)
}

async fn spawn_terminal_stream_server(
    tmp: &Path,
) -> (
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
    tokio::sync::oneshot::Receiver<SubmitWorkloadRequest>,
) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let material = mint_ephemeral_ca().expect("mint test TLS material");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test HTTPS listener");
    let endpoint = format!("https://{}", listener.local_addr().expect("listener address"));
    write_trust_triple(&tmp.join("conf"), &endpoint, &material)
        .expect("write test client trust triple");

    let tls = RustlsConfig::from_pem(
        material.server_leaf_cert_pem.into_bytes(),
        material.server_leaf_key_pem.into_bytes(),
    )
    .await
    .expect("load test server certificate");
    let (request_tx, request_rx) = tokio::sync::oneshot::channel();
    let app = Router::new()
        .route("/v1/workloads", post(stream_terminal_response))
        .with_state(Arc::new(Mutex::new(Some(request_tx))));
    let server = axum_server::from_tcp_rustls(listener, tls).serve(app.into_make_service());

    (tokio::spawn(server), request_rx)
}

/// S-SVM-23 — the detached `deploy` driving port sends exactly the existing
/// Service request and preserves every field in its selected VM driver arm.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[serial(workload_cgroup)]
async fn detached_service_deploy_forwards_vm_driver_without_parallel_request_shape() {
    let (handle, tmp) = spawn_server().await;
    let output = overdrive_cli::commands::deploy::deploy(DeployArgs {
        spec: write_vm_service(tmp.path()),
        config_path: config_path(tmp.path()),
    })
    .await
    .expect("detached VM Service deploy must receive the existing JSON acknowledgement");

    assert_eq!(output.workload_id, "vm-service");
    assert!(overdrive_cli::render::workload_submit_accepted(&output).contains("Accepted."));

    handle.shutdown().await.expect("clean server shutdown");
    assert_vm_driver_preserved(stored_vm_driver(tmp.path()).await);
}

/// S-SVM-24 — the streaming `deploy_streaming` driving port sends the same
/// existing Service request and retains its established terminal rendering and
/// exit-code result while preserving the selected VM driver arm.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[serial(workload_cgroup)]
async fn streaming_service_deploy_has_driver_projection_parity_with_detached_lane() {
    let tmp = TempDir::new().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("conf")).expect("create config directory");
    let (server, request_rx) = spawn_terminal_stream_server(tmp.path()).await;
    let config_path = config_path(tmp.path());
    let spec = write_vm_service(tmp.path());
    let output =
        overdrive_cli::commands::deploy::deploy_streaming(DeployArgs { spec, config_path })
            .await
            .expect("streaming VM Service deploy must consume the terminal response");

    assert_eq!(output.workload_id, "vm-service");
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.summary, "Service 'vm-service' was stopped by operator.\n");

    let request = request_rx.await.expect("capture the streaming request");
    let overdrive_core::api::submit::SubmitSpecInput::Service(service) = request.spec else {
        panic!("the streaming deploy lane must submit SubmitSpecInput::Service");
    };
    assert_vm_driver_input_preserved(service.driver);
    server.abort();
}
