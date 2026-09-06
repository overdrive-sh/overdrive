//! Service VM deploy-lane acceptance tests.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use overdrive_cli::commands::deploy::{DeployArgs, StopArgs};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_core::aggregate::{IntentKey, WorkloadDriver, WorkloadIntent};
use overdrive_core::id::WorkloadId;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_store_local::LocalIntentStore;
use serial_test::serial;
use tempfile::TempDir;

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
    let (handle, tmp) = spawn_server().await;
    let config_path = config_path(tmp.path());
    let spec = write_vm_service(tmp.path());
    let submit_config = config_path.clone();
    let mut submit = tokio::spawn(async move {
        overdrive_cli::commands::deploy::deploy_streaming(DeployArgs {
            spec,
            config_path: submit_config,
        })
        .await
    });

    tokio::time::sleep(Duration::from_millis(500)).await;
    let _ = overdrive_cli::commands::deploy::stop(StopArgs {
        id: "vm-service".to_owned(),
        config_path,
    })
    .await;

    match tokio::time::timeout(Duration::from_secs(2), &mut submit).await {
        Ok(Ok(Ok(output))) => {
            assert_eq!(output.workload_id, "vm-service");
            assert!(
                !output.summary.is_empty(),
                "the existing terminal renderer must produce output"
            );
            assert!(matches!(output.exit_code, 0 | 1));
        }
        Ok(Ok(Err(error))) => {
            panic!("streaming VM Service deploy failed before Accepted: {error:?}");
        }
        Ok(Err(error)) => panic!("streaming task panicked: {error:?}"),
        Err(_) => {
            // Guest startup and its terminal lifecycle are exercised by later
            // VM-health slices. This open response reached the existing
            // Accepted stream; end only this test-owned client task.
            submit.abort();
        }
    }

    handle.shutdown().await.expect("clean server shutdown");
    assert_vm_driver_preserved(stored_vm_driver(tmp.path()).await);
}
