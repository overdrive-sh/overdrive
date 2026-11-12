//! R-76 (ADR-0062 amendment 2026-11-10) — a stopped Service's last backend is
//! purged from the service map, and the outcome holds after a node restart.
//!
//! The production composition `run_server_with_obs_and_driver(..)` over the
//! same boot bundle every `serve` fixture uses (`SimDriver`, the Sim shared
//! guest-network owner, a `SimDataplaneComposer` over a `SimDataplane` the test
//! keeps a handle to). The Service is deployed and stopped, and allocations are
//! read, through the public HTTPS API only. Every allocation row, listener fact,
//! backend row and hydrator update is authored by production; the only reads of
//! simulated state are the dataplane port's own accessors
//! (`SimDataplane::service_vip_keys`, `SimDataplane::service_backends`).
//!
//! The listener facts the hydrator projects from survive a stop (R-76), so the
//! `ServiceMapHydrator` sees the stopped Service's zero-backend row and emits
//! the empty `DataplaneUpdateService` that removes the last backend. The
//! restart arm boots a second server over the same data directory: the
//! intent store and the observation store are the durable ones, the dataplane
//! is a fresh one (as after a real restart), and the stopped Service must not
//! be programmed again.
//!
//! Tier 1 (seeded `SimClock` advanced by the test). The convergence loop runs on
//! the injected clock.

#![allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::api::{AllocStateWire, AllocStatusResponse, SubmitWorkloadRequest};
use overdrive_control_plane::{
    BootAdapters, ServerConfig, ServerHandle, run_server_with_obs_and_driver,
};
use overdrive_core::aggregate::{DriverInput, ResourcesInput, VmInput};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::guest_network::GuestNetworkExecWiring;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::{SimDataplaneComposer, SimGuestDnsFactory};
use tempfile::TempDir;

use super::workload_lifecycle::wait::advance_and_settle;

/// The convergence tick cadence the injected clock is advanced by.
const TICK: Duration = Duration::from_millis(100);
/// Upper bound on injected ticks any one wait may take (30 s of logical time).
const MAX_TICKS: usize = 300;
/// Injected ticks the restart arm lets the booted node converge before it
/// asserts the stopped Service was not programmed again (10 s of logical time,
/// 100 reconcile passes).
const RESTART_SETTLE_TICKS: usize = 100;
const SERVICE: &str = "purge-svc";

fn client_trusting(ca_pem: &str) -> reqwest::Client {
    let cert = reqwest::Certificate::from_pem(ca_pem.as_bytes()).expect("parse CA PEM");
    reqwest::Client::builder()
        .add_root_certificate(cert)
        .https_only(true)
        .use_rustls_tls()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("build reqwest client")
}

fn read_ca_from_trust_triple(operator_config_dir: &std::path::Path) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    let config_path = operator_config_dir.join(".overdrive").join("config");
    let text = std::fs::read_to_string(&config_path)
        .unwrap_or_else(|error| panic!("read trust triple at {}: {error}", config_path.display()));
    let doc: toml::Value = toml::from_str(&text).expect("parse trust triple TOML");
    let ca_b64 = doc
        .get("contexts")
        .and_then(toml::Value::as_array)
        .and_then(|contexts| {
            contexts
                .iter()
                .find(|context| context.get("name").and_then(toml::Value::as_str) == Some("local"))
        })
        .and_then(|context| context.get("ca"))
        .and_then(toml::Value::as_str)
        .expect("[[contexts]] with name=\"local\" must carry a ca field");
    String::from_utf8(BASE64.decode(ca_b64).expect("base64 decode ca")).expect("ca PEM is UTF-8")
}

/// One lifetime of the node over a data directory that may outlive it.
struct Node {
    handle: ServerHandle,
    clock: Arc<SimClock>,
    dataplane: Arc<SimDataplane>,
    client: reqwest::Client,
    base: String,
}

impl Node {
    /// Boot over `dir`'s data and operator-config directories with a fresh
    /// `SimDataplane`, `SimDriver` and `SimClock`; the intent and observation
    /// stores are the ones the host bundle opens under the data directory, so
    /// a second boot over the same `dir` reads the first lifetime's state.
    async fn boot(dir: &TempDir) -> Self {
        let data_dir = dir.path().join("data");
        let operator_config_dir = dir.path().join("conf");
        std::fs::create_dir_all(&data_dir).expect("create data dir");
        std::fs::create_dir_all(&operator_config_dir).expect("create operator config dir");
        let clock = Arc::new(SimClock::new());
        let clock_port: Arc<dyn Clock> = clock.clone();
        let config = ServerConfig {
            tick_cadence: TICK,
            ..super::node_adapters::test_config(data_dir, operator_config_dir.clone())
        };
        let dataplane = Arc::new(SimDataplane::new());
        let mut boot: BootAdapters = super::node_adapters::boot_adapters(&config);
        boot.dataplane =
            Arc::new(SimDataplaneComposer::new(Arc::clone(&dataplane), Ipv4Addr::LOCALHOST));
        boot.guest_dns = Arc::new(SimGuestDnsFactory::default());
        boot.clock = Arc::clone(&clock_port);
        let driver: Arc<dyn Driver> =
            Arc::new(SimDriver::with_clock(DriverType::Vm, Arc::clone(&clock_port)));
        let wiring = GuestNetworkExecWiring::new(Arc::clone(&clock_port));
        let handle = run_server_with_obs_and_driver(
            config,
            boot,
            driver,
            Arc::new(SimSharedGuestNetworkOwner::default()),
            wiring,
        )
        .await
        .expect("production boot");
        let bound = handle.local_addr().await.expect("server bound address");
        let client = client_trusting(&read_ca_from_trust_triple(&operator_config_dir));
        let base = format!("https://localhost:{}", bound.port());
        Self { handle, clock, dataplane, client, base }
    }

    async fn submit_service(&self) {
        let spec = SubmitSpecInput::Service(ServiceSpecInput {
            id: SERVICE.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 134_217_728 },
            driver: DriverInput::Vm(VmInput {
                command: "/bin/sleep".to_owned(),
                args: vec!["3600".to_owned()],
                kernel: "/kernel".to_owned(),
                rootfs: "/rootfs".to_owned(),
            }),
            listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
            startup_probes: vec![],
            readiness_probes: vec![],
            liveness_probes: vec![],
        });
        let response = self
            .client
            .post(format!("{}/v1/workloads", self.base))
            .json(&SubmitWorkloadRequest { spec })
            .send()
            .await
            .expect("submit reaches the public API");
        assert!(response.status().is_success(), "submit: {response:?}");
    }

    async fn stop_service(&self) {
        let response = self
            .client
            .post(format!("{}/v1/workloads/{SERVICE}/stop", self.base))
            .send()
            .await
            .expect("stop reaches the public API");
        assert!(response.status().is_success(), "stop {SERVICE}: {response:?}");
    }

    async fn allocs(&self) -> AllocStatusResponse {
        let response = self
            .client
            .get(format!("{}/v1/allocs?job={SERVICE}", self.base))
            .send()
            .await
            .expect("GET /v1/allocs reaches the public API");
        assert!(response.status().is_success(), "GET /v1/allocs: {response:?}");
        response.json().await.expect("AllocStatusResponse body")
    }

    /// The service map's VIPs, as the dataplane port reports them.
    fn programmed_vips(&self) -> Vec<Ipv4Addr> {
        self.dataplane.service_vip_keys()
    }

    /// Advance the injected clock tick by tick until `done` holds.
    async fn tick_until(&self, what: &str, done: impl Fn(&Self) -> bool) {
        for _ in 0..MAX_TICKS {
            if done(self) {
                return;
            }
            advance_and_settle(&self.clock, TICK).await;
        }
        panic!(
            "{what}: not reached within {MAX_TICKS} ticks; allocs {:?}, programmed VIPs {:?}",
            self.allocs().await,
            self.programmed_vips()
        );
    }

    async fn settle(&self, ticks: usize) {
        for _ in 0..ticks {
            advance_and_settle(&self.clock, TICK).await;
        }
    }

    async fn shutdown(self) {
        let Self { handle, clock, .. } = self;
        let mut shutdown = std::pin::pin!(handle.shutdown(Duration::from_secs(2)));
        let result = loop {
            tokio::select! {
                biased;
                result = &mut shutdown => break result,
                () = tokio::time::sleep(Duration::from_millis(5)) => clock.tick(TICK),
            }
        };
        if let Err(error) = result {
            eprintln!("diagnostic=node_shutdown_error error={error}");
        }
    }
}

/// Outcome anchor: OUT-PIG-STOPPED-SERVICE-PURGE
/// R-76 boundary obligation — a stopped Service's last backend is purged from
/// the service map, and a restart of the node preserves the same outcome.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given a Service deployed through the public API whose one replica is Running
///   and programmed in the service map
/// When the operator stops the Service
/// Then the service map holds no backend for the Service's VIP once the
///      replica is Terminated
///  And after the node restarts over the same state, the stopped Service is
///      Terminated and the service map still holds no backend for any VIP.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stopped_services_last_backend_is_purged_from_the_service_map_and_stays_purged_after_a_node_restart()
 {
    let dir = TempDir::new().expect("tempdir");
    let first = Node::boot(&dir).await;

    // GIVEN the Service is Running and programmed.
    first.submit_service().await;
    first
        .tick_until("the Service is programmed in the service map", |node| {
            node.programmed_vips().iter().any(|vip| node.dataplane.service_backends(*vip).is_some())
        })
        .await;
    let vips = first.programmed_vips();
    assert_eq!(vips.len(), 1, "exactly the one Service is programmed: {vips:?}");
    let vip = vips[0];
    let backends = first.dataplane.service_backends(vip).expect("the VIP carries its backend");
    assert_eq!(backends.len(), 1, "the one replica is the Service's one backend: {backends:?}");

    // WHEN the operator stops the Service.
    first.stop_service().await;

    // THEN the last backend is purged from the service map.
    first
        .tick_until("the stopped Service's last backend leaves the service map", |node| {
            node.dataplane.service_backends(vip).is_none()
        })
        .await;
    let stopped = first.allocs().await;
    assert!(
        stopped.rows.iter().all(|row| row.state == AllocStateWire::Terminated),
        "the stopped Service's replica is Terminated: {stopped:?}"
    );
    assert_eq!(
        first.dataplane.service_backends(vip),
        None,
        "no backend remains for the stopped Service's VIP"
    );
    first.shutdown().await;

    // AND a restart over the same state preserves the outcome.
    let second = Node::boot(&dir).await;
    second.settle(RESTART_SETTLE_TICKS).await;
    let restarted = second.allocs().await;
    assert!(
        restarted.rows.iter().all(|row| row.state == AllocStateWire::Terminated),
        "after the restart the stopped Service has no live replica: {restarted:?}"
    );
    for programmed in second.programmed_vips() {
        assert_eq!(
            second.dataplane.service_backends(programmed),
            None,
            "after the restart no backend is programmed for the stopped Service (VIP {programmed})"
        );
    }
    second.shutdown().await;
}
