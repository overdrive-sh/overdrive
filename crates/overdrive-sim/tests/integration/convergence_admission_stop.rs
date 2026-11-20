//! Admission bound of the production convergence loop under pending Stop effects.
//!
//! The loop (`spawn_convergence_loop`) runs at most `CONVERGENCE_MAX_IN_FLIGHT`
//! (8) evaluations at once, and closing admission at shutdown drains the
//! in-flight evaluations to completion without admitting another.
//!
//! The scenario drives causes through the production entry points: eight
//! workloads are submitted and stopped over the operator HTTPS surface of the
//! real `run_server_with_obs_and_driver` composition. The only substituted edge
//! is the `Driver` port, whose `stop` is parked to stand in for external VMM
//! latency. No row, broker entry or evaluation is fabricated. The oracle is
//! the loop's `convergence.evaluation.admitted` / `completed` /
//! `drain.completed` events plus the Driver boundary and the observation store.
//!
//! The trace cursor is taken before the first stop is submitted: a submit wakes
//! the loop at once, so a cursor taken after the submissions would miss every
//! admission.
//!
//! The only time source is the fixture's `SimClock`, advanced by the harness;
//! the seed is printed for reproduction.
//!
//! CONTRACT_SHAPE: bounded-change.

#![allow(clippy::print_stderr, clippy::too_many_lines, clippy::large_futures)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use overdrive_control_plane::api::{SubmitWorkloadRequest, SubmitWorkloadResponse};
use overdrive_control_plane::dataplane_config::DataplaneConfig;
use overdrive_control_plane::{
    BootAdapters, NodeAdapters, ServerConfig, run_server_with_obs_and_driver,
};
use overdrive_core::aggregate::{DriverInput, JobSpecInput, ResourcesInput};
use overdrive_core::api::submit::SubmitSpecInput;
use overdrive_core::guest_transport::NodeVsockPortConfig;
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, ExitEvent,
    Resources,
};
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use overdrive_sim::adapters::{
    SimCgroupFs, SimDataplaneComposer, SimGuestDnsFactory, SimKek, SimMtlsIntercept,
    clock::SimClock, dataplane::SimDataplane, driver::SimDriver,
    guest_network::SimSharedGuestNetworkOwner, observation_store::SimObservationStore,
};
use parking_lot::Mutex;
use tokio::sync::{Semaphore, mpsc};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

/// The loop's admission bound (`CONVERGENCE_MAX_IN_FLIGHT`).
const ADMISSION_BOUND: usize = 8;
const SEED: u64 = 283_001;

#[derive(Clone, Debug)]
struct CapturedEvent {
    name: String,
    fields: BTreeMap<String, String>,
}

impl CapturedEvent {
    fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(|value| value.trim_matches('"'))
    }

    fn u64_field(&self, name: &str) -> Option<u64> {
        self.field(name)?.parse().ok()
    }

    /// The evaluation this event reports: reconciler, target and tick.
    fn evaluation(&self) -> (String, String, u64) {
        (
            self.field("reconciler").expect("evaluation event names its reconciler").to_owned(),
            self.field("target").expect("evaluation event names its target").to_owned(),
            self.u64_field("tick").expect("evaluation event carries its tick"),
        )
    }
}

#[derive(Clone, Default)]
struct CaptureLayer(Arc<std::sync::Mutex<Vec<CapturedEvent>>>);

struct FieldVisitor(BTreeMap<String, String>);

impl tracing::field::Visit for FieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor(BTreeMap::new());
        event.record(&mut visitor);
        self.0
            .lock()
            .expect("trace events mutex")
            .push(CapturedEvent { name: event.metadata().name().to_owned(), fields: visitor.0 });
    }
}

impl CaptureLayer {
    fn cursor(&self) -> usize {
        self.0.lock().expect("trace events mutex").len()
    }

    fn since(&self, cursor: usize) -> Vec<CapturedEvent> {
        self.0.lock().expect("trace events mutex")[cursor..].to_vec()
    }
}

/// The `Driver` port with `stop` parked for the slow workloads until released.
struct ParkedStopDriver {
    inner: SimDriver,
    stop_entered: mpsc::UnboundedSender<AllocationId>,
    release: Semaphore,
    started: Mutex<Vec<AllocationId>>,
    stop_returns: Mutex<Vec<AllocationId>>,
}

#[async_trait]
impl Driver for ParkedStopDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Vm
    }
    async fn start(
        &self,
        spec: &AllocationSpec,
        supersession: &dyn overdrive_core::traits::LaunchSupersession,
    ) -> Result<AllocationHandle, DriverError> {
        self.started.lock().push(spec.alloc.clone());
        self.inner.start(spec, supersession).await
    }
    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        self.inner.stop(handle).await?;
        if handle.alloc.as_str().starts_with("alloc-slow") {
            self.stop_entered.send(handle.alloc.clone()).expect("entry receiver outlives the run");
            self.release.acquire().await.expect("release semaphore stays open").forget();
        }
        self.stop_returns.lock().push(handle.alloc.clone());
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
    fn take_exit_receiver(&self) -> Option<mpsc::Receiver<ExitEvent>> {
        self.inner.take_exit_receiver()
    }
    fn release_supervision(&self, alloc: &AllocationId) {
        self.inner.release_supervision(alloc);
    }
}

fn sim_boot(
    config: &ServerConfig,
    clock: &Arc<SimClock>,
    obs: Arc<SimObservationStore>,
) -> BootAdapters {
    let mut boot =
        NodeAdapters::host(config, Arc::new(SimCgroupFs::new()), Arc::new(SimKek::for_boot()))
            .expect("the host bundle opens its stores under the fixture's data_dir")
            .boot;
    boot.clock = clock.clone();
    boot.observation = obs;
    boot.vm_host_state = Arc::new(overdrive_sim::adapters::vm_host_state::SimVmHostState::new());
    boot.dataplane = Arc::new(SimDataplaneComposer::new(
        Arc::new(SimDataplane::new()),
        std::net::Ipv4Addr::LOCALHOST,
    ));
    boot.mtls.intercept = Arc::new(SimMtlsIntercept::new());
    boot.guest_dns = Arc::new(SimGuestDnsFactory::default());
    boot
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
            driver: DriverInput::Vm(overdrive_core::aggregate::VmInput {
                volume: None,
                command: "/bin/sleep".to_owned(),
                args: vec!["3600".to_owned()],
                kernel: "/kernel".to_owned(),
                rootfs: "/rootfs".to_owned(),
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

async fn advance(clock: &SimClock) {
    clock.tick(Duration::from_millis(100 + SEED % 7));
    // Real HTTPS and redb are only integration-host scheduling; their
    // wall-clock settle is not a simulated latency result.
    tokio::time::sleep(Duration::from_millis(5)).await;
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

/// The number of evaluations in flight after each admission or completion, in
/// event order, replayed from the loop's own events.
fn in_flight_levels(events: &[CapturedEvent]) -> Vec<usize> {
    let mut in_flight = BTreeSet::new();
    let mut levels = Vec::new();
    for event in events {
        match event.name.as_str() {
            "convergence.evaluation.admitted" => {
                assert!(in_flight.insert(event.evaluation()), "an evaluation was admitted twice");
            }
            "convergence.evaluation.completed" => {
                assert!(
                    in_flight.remove(&event.evaluation()),
                    "an evaluation completed without being admitted"
                );
            }
            _ => continue,
        }
        levels.push(in_flight.len());
    }
    levels
}

/// With eight stop effects pending the loop holds exactly eight evaluations in
/// flight and leaves the ninth workload's evaluation unadmitted. Closing
/// admission then drains the eight owned evaluations to completion, admits no
/// ninth, and reports one drain event.
#[tokio::test(flavor = "current_thread")]
async fn closing_admission_drains_eight_pending_stops_without_admitting_a_ninth() {
    let capture = CaptureLayer::default();
    let _subscriber = tracing::subscriber::set_default(Registry::default().with(capture.clone()));
    eprintln!("convergence-admission seed={SEED} effect=stop held={ADMISSION_BOUND}");

    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(SimClock::new());
    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("local").unwrap(), SEED));
    let (stop_entered, mut entries) = mpsc::unbounded_channel();
    let driver = Arc::new(ParkedStopDriver {
        inner: SimDriver::with_clock(DriverType::Vm, clock.clone()),
        stop_entered,
        release: Semaphore::new(0),
        started: Mutex::new(Vec::new()),
        stop_returns: Mutex::new(Vec::new()),
    });
    let config_dir = directory.path().join("operator");
    let config = ServerConfig {
        data_dir: directory.path().join("data"),
        operator_config_dir: config_dir.clone(),
        dataplane: Some(DataplaneConfig { client_iface: "lo".into(), backend_iface: "lo".into() }),
        vsock_ports: NodeVsockPortConfig::kernel_assigned(),
        ..ServerConfig::new()
    };
    let boot = sim_boot(&config, &clock, obs.clone());
    let wiring = overdrive_core::guest_network::GuestNetworkExecWiring::new(clock.clone());
    let server = run_server_with_obs_and_driver(
        config,
        boot,
        driver.clone(),
        Arc::new(SimSharedGuestNetworkOwner::default()),
        wiring,
    )
    .await
    .unwrap();
    let base = format!("https://localhost:{}", server.local_addr().await.unwrap().port());
    let client = client(&config_dir);

    // Eight workloads reach Running through the healthy path.
    for index in 0..ADMISSION_BOUND {
        submit(&client, &base, &format!("slow{index}")).await;
    }
    for _ in 0..100 {
        advance(&clock).await;
    }
    for index in 0..ADMISSION_BOUND {
        assert!(
            running(&obs, &format!("slow{index}")).await,
            "seed={SEED}: healthy setup failed for slow{index}"
        );
    }

    // The cursor precedes the first stop: each submit wakes the loop at once.
    let stop_cursor = capture.cursor();
    for index in 0..ADMISSION_BOUND {
        let id = format!("slow{index}");
        let response = client.post(format!("{base}/v1/workloads/{id}/stop")).send().await.unwrap();
        assert!(response.status().is_success(), "seed={SEED}: stop rejected for {id}");
    }
    let mut parked = BTreeSet::new();
    for _ in 0..100 {
        advance(&clock).await;
        while let Ok(alloc) = entries.try_recv() {
            parked.insert(alloc);
        }
        if parked.len() == ADMISSION_BOUND {
            break;
        }
    }
    let expected_allocations: BTreeSet<_> = (0..ADMISSION_BOUND)
        .map(|index| AllocationId::new(&format!("alloc-slow{index}-0")).unwrap())
        .collect();
    assert_eq!(parked, expected_allocations, "seed={SEED}: all eight stops reach the Driver");

    // A ninth workload arrives while the eight stops are pending.
    submit(&client, &base, "independent").await;
    for _ in 0..50 {
        advance(&clock).await;
    }
    assert!(!running(&obs, "independent").await, "seed={SEED}: the ninth evaluation progressed");

    let held_events = capture.since(stop_cursor);
    let levels = in_flight_levels(&held_events);
    assert_eq!(
        levels.iter().max().copied(),
        Some(ADMISSION_BOUND),
        "seed={SEED}: eight evaluations in flight at once, never more"
    );
    assert!(
        held_events.iter().filter(|event| event.name == "convergence.evaluation.admitted").all(
            |event| event.u64_field("capacity") == Some(ADMISSION_BOUND as u64)
                && event.u64_field("active").is_some_and(|active| active <= ADMISSION_BOUND as u64)
        ),
        "seed={SEED}: admission events report the eight-way bound"
    );
    assert!(
        held_events
            .iter()
            .filter(|event| event.name == "convergence.evaluation.admitted")
            .all(|event| event.field("target") != Some("workload/independent")),
        "seed={SEED}: the ninth workload was admitted while eight evaluations were in flight"
    );

    // Close admission while all eight stops are parked.
    let close_cursor = capture.cursor();
    let shutdown = tokio::spawn(server.shutdown(Duration::from_secs(1)));
    for _ in 0..10 {
        advance(&clock).await;
    }
    assert!(!shutdown.is_finished(), "seed={SEED}: shutdown returned before the held stops");
    driver.release.add_permits(ADMISSION_BOUND);
    tokio::time::timeout(Duration::from_secs(10), shutdown).await.unwrap().unwrap().unwrap();

    let drain_events = capture.since(close_cursor);
    assert!(
        drain_events.iter().all(|event| event.name != "convergence.evaluation.admitted"),
        "seed={SEED}: an evaluation was admitted after admission closed"
    );
    let completed_after_close: BTreeSet<_> = drain_events
        .iter()
        .filter(|event| event.name == "convergence.evaluation.completed")
        .map(|event| event.field("target").unwrap_or_default().to_owned())
        .collect();
    let expected_targets: BTreeSet<_> =
        (0..ADMISSION_BOUND).map(|index| format!("workload/slow{index}")).collect();
    assert_eq!(
        completed_after_close, expected_targets,
        "seed={SEED}: the drain completes exactly the eight owned stop evaluations"
    );
    let drains: Vec<_> =
        drain_events.iter().filter(|event| event.name == "convergence.drain.completed").collect();
    assert_eq!(drains.len(), 1, "seed={SEED}: exactly one drain report");
    assert_eq!(drains[0].u64_field("admitted_at_close"), Some(ADMISSION_BOUND as u64));
    assert_eq!(drains[0].u64_field("completed_during_drain"), Some(ADMISSION_BOUND as u64));
    let drain_index = drain_events
        .iter()
        .position(|event| event.name == "convergence.drain.completed")
        .expect("the single drain report exists");
    assert!(
        drain_events
            .iter()
            .enumerate()
            .filter(|(_, event)| event.name == "convergence.evaluation.completed")
            .all(|(index, _)| index < drain_index),
        "seed={SEED}: every completion precedes the drain report"
    );
    assert_eq!(
        in_flight_levels(&capture.since(stop_cursor)).last().copied(),
        Some(0),
        "seed={SEED}: nothing remains in flight after the drain"
    );

    // The Driver and the observation store never saw the ninth workload.
    let returned: BTreeSet<_> = driver.stop_returns.lock().iter().cloned().collect();
    assert_eq!(returned, expected_allocations, "seed={SEED}: every held stop returned");
    assert!(
        driver.started.lock().iter().all(|alloc| !alloc.as_str().starts_with("alloc-independent")),
        "seed={SEED}: the ninth workload reached the Driver"
    );
    assert!(
        obs.alloc_status_rows()
            .await
            .unwrap()
            .iter()
            .all(|row| row.workload_id.as_str() != "independent"),
        "seed={SEED}: the ninth workload acquired an allocation"
    );
    eprintln!(
        "convergence-admission seed={SEED} max_in_flight={} drained={} ninth_admissions=0",
        levels.iter().max().copied().unwrap_or(0),
        completed_after_close.len()
    );
}
