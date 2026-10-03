//! S-ND295-05A (in-process half) — a server restarted after being killed
//! starts with an empty guest-attachment pool (GH #295, D-295-R6: one pool per
//! server, FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (one pool per server: the doc-hidden pool and its crate-private operations)).
//!
//! In-process lane (Lima root, `integration-tests`): two boots of the
//! production composition `run_server_with_obs_and_driver(ServerConfig::new(kek,
//! mtls_intercept, guest_dns), obs, driver, vm_host_state, shared_guest_network,
//! guest_network_exec, vm_cgroups)` (FD § "EXEC-close linearization" (the `run_server_with_obs_and_driver(s)` signatures)) on the same data and
//! operator roots, each with a `SimDriver`, a `SimSharedGuestNetworkOwner`,
//! `SimGuestDnsFactory`, and `vm_cgroups` over `SimCgroupFs`. The first server
//! is abandoned with `ServerHandle::kill_for_test`, the way process death would
//! abandon it; the observation store survives the kill, as the production
//! store does.
//!
//! The assigned guest address is read from the guest-network assignment in the
//! spec each `SimDriver` received (`SimDriver::started_specs()`, the C-295-A
//! handoff); workloads are deployed through the public HTTPS API only.

#![allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::api::SubmitWorkloadRequest;
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server_with_obs_and_driver};
use overdrive_core::aggregate::{DriverInput, JobSpecInput, ResourcesInput, VmInput};
use overdrive_core::api::submit::SubmitSpecInput;
use overdrive_core::guest_network::GuestNetworkExecWiring;
use overdrive_core::id::NodeId;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{AllocationSpec, Driver, DriverType};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::vm_host_state::SimVmHostState;
use overdrive_sim::adapters::{SimCgroupFs, SimGuestDnsFactory, SimKek, SimMtlsIntercept};
use overdrive_worker::cgroup_manager::CgroupManager;
use tempfile::TempDir;

use super::workload_lifecycle::wait::advance_and_settle;

/// The convergence tick cadence the injected clock is advanced by.
const TICK: Duration = Duration::from_millis(100);
/// Upper bound on injected ticks a start may take (30 s of logical time).
const MAX_TICKS: usize = 300;
/// The smallest free guest address of the node prefix: the first assignment of
/// an empty pool (`100.95.0.1` is the shared gateway).
const FIRST_GUEST_ADDRESS: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 2);

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

fn read_ca_from_trust_triple(operator_config_dir: &Path) -> String {
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

/// One server over `roots`, with fresh process-local adapters and the shared,
/// surviving observation store.
struct Server {
    handle: ServerHandle,
    clock: Arc<SimClock>,
    driver: Arc<SimDriver>,
    client: reqwest::Client,
    base: String,
}

impl Server {
    async fn boot(roots: &Path, obs: Arc<dyn ObservationStore>) -> Self {
        let data_dir = roots.join("data");
        let operator_config_dir = roots.join("conf");
        std::fs::create_dir_all(&data_dir).expect("create data dir");
        std::fs::create_dir_all(&operator_config_dir).expect("create operator config dir");
        let clock = Arc::new(SimClock::new());
        let clock_port: Arc<dyn Clock> = clock.clone();
        let config = ServerConfig {
            bind: "127.0.0.1:0".parse().expect("parse bind addr"),
            data_dir,
            operator_config_dir: operator_config_dir.clone(),
            tick_cadence: TICK,
            clock: Arc::clone(&clock_port),
            dataplane: Some(super::dataplane_lo::lo_dataplane_config()),
            dataplane_override: Some(Arc::new(SimDataplane::new())),
            ..ServerConfig::new(
                Arc::new(SimKek::for_boot()),
                Arc::new(SimMtlsIntercept::new()),
                Arc::new(SimGuestDnsFactory::default()),
            )
        };
        let driver = Arc::new(SimDriver::with_clock(DriverType::Vm, Arc::clone(&clock_port)));
        let driver_port: Arc<dyn Driver> = driver.clone();
        let handle = run_server_with_obs_and_driver(
            config,
            obs,
            driver_port,
            Arc::new(SimVmHostState::new()),
            Arc::new(SimSharedGuestNetworkOwner::default()),
            GuestNetworkExecWiring::new(Arc::clone(&clock_port)),
            CgroupManager::new(PathBuf::from("/sys/fs/cgroup"), Arc::new(SimCgroupFs::new())),
        )
        .await
        .expect("production boot");
        let bound = handle.local_addr().await.expect("server bound address");
        let client = client_trusting(&read_ca_from_trust_triple(&operator_config_dir));
        let base = format!("https://localhost:{}", bound.port());
        Self { handle, clock, driver, client, base }
    }

    async fn deploy_vm_job(&self, id: &str) {
        let spec = SubmitSpecInput::Job(JobSpecInput {
            id: id.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 134_217_728 },
            driver: DriverInput::Vm(VmInput {
                command: "/bin/sleep".to_owned(),
                args: vec!["3600".to_owned()],
                kernel: "/kernel".to_owned(),
                rootfs: "/rootfs".to_owned(),
            }),
        });
        let response = self
            .client
            .post(format!("{}/v1/workloads", self.base))
            .json(&SubmitWorkloadRequest { spec })
            .send()
            .await
            .expect("submit reaches the public API");
        assert!(response.status().is_success(), "submit {id}: {response:?}");
    }

    /// Advance the injected clock until this server's driver has received its
    /// first start, returning every spec it received.
    async fn await_first_start(&self, what: &str) -> Vec<AllocationSpec> {
        for _ in 0..MAX_TICKS {
            let started = self.driver.started_specs();
            if !started.is_empty() {
                return started;
            }
            advance_and_settle(&self.clock, TICK).await;
        }
        panic!("{what}: the driver received no start within {MAX_TICKS} ticks");
    }

    /// Drive `work` to completion while the injected clock keeps advancing, so
    /// no clock-bounded wait inside it parks forever.
    async fn ticking<F: std::future::Future>(clock: &SimClock, work: F) -> F::Output {
        let mut work = std::pin::pin!(work);
        loop {
            tokio::select! {
                biased;
                output = &mut work => return output,
                () = tokio::time::sleep(Duration::from_millis(5)) => clock.tick(TICK),
            }
        }
    }
}

fn first_address(started: &[AllocationSpec]) -> Option<Ipv4Addr> {
    started.first().and_then(|spec| spec.network.as_ref()).map(|network| network.address)
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH
/// S-ND295-05A — Admission refuses at the cap over held leases, one pool per server
/// CONTRACT_SHAPE: bounded-change.
///
/// A server holding the node's first lease is killed; the next server booted on
/// the same roots starts with an empty pool, so the first guest-network
/// assignment it makes is again the smallest free address, `100.95.0.2`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_killed_restart_starts_with_an_empty_pool() {
    let roots = TempDir::new().expect("tempdir");
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));

    // GIVEN a server that holds the node's first lease.
    let first = Server::boot(roots.path(), Arc::clone(&obs)).await;
    first.deploy_vm_job("pool-before-kill").await;
    let before = first.await_first_start("the first server starts its workload").await;
    assert_eq!(
        first_address(&before),
        Some(FIRST_GUEST_ADDRESS),
        "precondition: an empty pool assigns the smallest free address first; specs {before:?}"
    );

    // WHEN that server is killed and a new one boots on the same roots.
    let Server { handle, clock, .. } = first;
    let _residue = Server::ticking(&clock, handle.kill_for_test()).await;
    let second = Server::boot(roots.path(), Arc::clone(&obs)).await;
    second.deploy_vm_job("pool-after-kill").await;
    let after = second.await_first_start("the restarted server starts a workload").await;

    // THEN the restarted server's first assignment is the smallest free
    // address: the killed server's lease did not survive into its pool.
    assert_eq!(
        first_address(&after),
        Some(FIRST_GUEST_ADDRESS),
        "a server restarted after being killed starts with an empty pool; specs {after:?}"
    );

    let Server { handle, clock, .. } = second;
    if let Err(error) = Server::ticking(&clock, handle.shutdown(Duration::from_secs(2))).await {
        eprintln!("diagnostic=server_shutdown_error error={error}");
    }
}
