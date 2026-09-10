//! Issue-283 diagnostic: the actual broker/convergence task under slow Driver effects.
//! <!-- DES-ENFORCEMENT : exempt -->
//! No terminal rows, private lifecycle state, or broker schedule are fabricated.
//! The Driver port substitutes external process latency; native probes own VMM timing.
#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::print_stderr, clippy::too_many_lines, clippy::large_futures)]
#![allow(
    clippy::doc_markdown,
    reason = "the required per-test CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use async_trait::async_trait;
use base64::Engine;
use overdrive_control_plane::api::{SubmitWorkloadRequest, SubmitWorkloadResponse};
use overdrive_control_plane::dataplane_config::DataplaneConfig;
use overdrive_control_plane::{ServerConfig, run_server_with_obs_and_driver};
use overdrive_core::aggregate::{DriverInput, ExecInput, JobSpecInput, ResourcesInput};
use overdrive_core::api::submit::SubmitSpecInput;
use overdrive_core::id::NodeId;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, Resources,
};
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use overdrive_sim::adapters::{
    SimKek, clock::SimClock, dataplane::SimDataplane, driver::SimDriver,
    observation_store::SimObservationStore,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Notify, mpsc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Effect {
    Healthy,
    Start,
    Stop,
}

struct DelayedDriver {
    inner: SimDriver,
    effect: Effect,
    entered: mpsc::UnboundedSender<Effect>,
    release: Notify,
}

#[async_trait]
impl Driver for DelayedDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Exec
    }
    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        if self.effect == Effect::Start && spec.alloc.as_str() == "alloc-slow-0" {
            self.entered.send(Effect::Start).unwrap();
            self.release.notified().await;
        }
        self.inner.start(spec).await
    }
    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        self.inner.stop(handle).await?;
        if self.effect == Effect::Stop && handle.alloc.as_str() == "alloc-slow-0" {
            self.entered.send(Effect::Stop).unwrap();
            self.release.notified().await;
        }
        Ok(())
    }
    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        self.inner.status(handle).await
    }
    async fn resize(
        &self,
        handle: &AllocationHandle,
        resources: Resources,
    ) -> Result<(), DriverError> {
        self.inner.resize(handle, resources).await
    }
    async fn release_for_exit_emission(&self, handle: &AllocationHandle) {
        self.inner.release_for_exit_emission(handle).await;
    }
    fn release_supervision(&self, alloc: &overdrive_core::id::AllocationId) {
        self.inner.release_supervision(alloc);
    }
}

fn client(config_dir: &std::path::Path) -> reqwest::Client {
    let contents = std::fs::read_to_string(config_dir.join(".overdrive/config")).unwrap();
    let config: toml::Value = toml::from_str(&contents).unwrap();
    let encoded = config["contexts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"].as_str() == Some("local"))
        .unwrap()["ca"]
        .as_str()
        .unwrap();
    let pem = base64::engine::general_purpose::STANDARD.decode(encoded).unwrap();
    reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(&pem).unwrap())
        .https_only(true)
        .use_rustls_tls()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

async fn submit(client: &reqwest::Client, base: &str, id: &str) {
    let body = SubmitWorkloadRequest {
        spec: SubmitSpecInput::Job(JobSpecInput {
            id: id.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            driver: DriverInput::Exec(ExecInput {
                command: "/bin/sleep".to_owned(),
                args: vec!["3600".to_owned()],
            }),
        }),
    };
    let response = client.post(format!("{base}/v1/workloads")).json(&body).send().await.unwrap();
    assert!(response.status().is_success(), "submit {id}: {response:?}");
    let _: SubmitWorkloadResponse = response.json().await.unwrap();
}

async fn running(obs: &SimObservationStore, id: &str) -> bool {
    obs.alloc_status_rows()
        .await
        .unwrap()
        .iter()
        .any(|row| row.workload_id.as_str() == id && row.state == AllocState::Running)
}

async fn advance(clock: &SimClock, seed: u64) {
    clock.tick(Duration::from_millis(100 + seed % 7));
    // Real HTTPS/redb are only integration-host scheduling. Their wall-clock
    // settle is not counted as a simulated latency result or product KPI.
    tokio::time::sleep(Duration::from_millis(5)).await;
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

async fn drive(effect: Effect) {
    let seed = 283_001;
    eprintln!("issue283 seed={seed} effect={effect:?}");
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(SimClock::new());
    let node = NodeId::new("local").unwrap();
    let obs = Arc::new(SimObservationStore::single_peer(node, seed));
    let (entered, mut entries) = mpsc::unbounded_channel();
    let driver = Arc::new(DelayedDriver {
        inner: SimDriver::with_clock(DriverType::Exec, clock.clone()),
        effect,
        entered,
        release: Notify::new(),
    });
    let config_dir = directory.path().join("operator");
    let config = ServerConfig {
        data_dir: directory.path().join("data"),
        operator_config_dir: config_dir.clone(),
        clock: clock.clone(),
        dataplane: Some(DataplaneConfig { client_iface: "lo".into(), backend_iface: "lo".into() }),
        dataplane_override: Some(Arc::new(SimDataplane::new())),
        ..ServerConfig::new(Arc::new(SimKek::for_boot()))
    };
    let server = run_server_with_obs_and_driver(config, obs.clone(), driver.clone()).await.unwrap();
    let bound = server.local_addr().await.unwrap();
    let client = client(&config_dir);
    let base = format!("https://localhost:{}", bound.port());
    submit(&client, &base, "slow").await;
    if effect != Effect::Start {
        for _ in 0..50 {
            advance(&clock, seed).await;
            if running(&obs, "slow").await {
                break;
            }
        }
        assert!(running(&obs, "slow").await, "seed={seed}: initial healthy start failed");
    }
    if effect == Effect::Stop {
        let response = client.post(format!("{base}/v1/workloads/slow/stop")).send().await.unwrap();
        assert!(response.status().is_success(), "stop: {response:?}");
    }
    if effect != Effect::Healthy {
        let mut observed = None;
        for _ in 0..50 {
            advance(&clock, seed).await;
            if let Ok(event) = entries.try_recv() {
                observed = Some(event);
                break;
            }
        }
        assert_eq!(observed, Some(effect), "seed={seed}: real owner never entered slow effect");
    }
    submit(&client, &base, "independent").await;
    let mut progressed_while_held = false;
    for _ in 0..10 {
        advance(&clock, seed).await;
        // This public query additionally proves the HTTP owner remains live.
        let response = client.get(format!("{base}/v1/cluster/info")).send().await.unwrap();
        assert!(response.status().is_success());
        progressed_while_held |= running(&obs, "independent").await;
    }
    driver.release.notify_one();
    let mut progressed_after_release = progressed_while_held;
    for _ in 0..100 {
        advance(&clock, seed).await;
        progressed_after_release |= running(&obs, "independent").await;
        if progressed_after_release {
            break;
        }
    }
    // Cooperative production shutdown, never abort the convergence owner.
    server.shutdown(Duration::from_secs(1)).await.unwrap();
    eprintln!(
        "issue283 seed={seed} effect={effect:?} independent_running_while_held={progressed_while_held} independent_running_after_release={progressed_after_release} shutdown=joined"
    );
    assert!(progressed_after_release, "seed={seed}: recovery/control must progress");
    assert!(
        progressed_while_held,
        "seed={seed}: independent workload cannot progress while {effect:?} holds the production convergence owner"
    );
}

/// CONTRACT_SHAPE: bounded-change.
/// Independent convergence while a start effect is pending.
#[tokio::test]
async fn slow_start_does_not_block_independent_convergence() {
    drive(Effect::Start).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// Independent convergence while a stop effect is pending.
#[tokio::test]
async fn slow_stop_does_not_block_independent_convergence() {
    drive(Effect::Stop).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// Healthy complete-owner convergence control.
#[tokio::test]
async fn healthy_driver_control_progresses() {
    drive(Effect::Healthy).await;
}
