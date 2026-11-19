//! Slice 01 acceptance — a deploy recovers by itself after a transient hydrate
//! error, a View-write error or a failed dataplane update, because the runtime
//! re-evaluates every managed target of every reconciler each period
//! (feature `reconciler-convergence-guarantees`; ADR-0321 decisions 2 and 3;
//! feature delta D-1, D-3).
//!
//! The production composition `run_server_with_obs_and_driver(..)` over the
//! same boot bundle every `serve` fixture uses. A workload is deployed and read
//! back through the public HTTPS API only. Faults are injected at existing
//! driven-port seams (the Sim observation store's read failure, the Sim view
//! store's `inject_fsync_failure`, the Sim dataplane's scripted
//! `update_service` failure); production authors every allocation row, backend
//! row and hydrator result. No test installs, binds or programs anything that
//! production does not.
//!
//! The convergence loop runs on the boot's one injected `SimClock`
//! (`BootAdapters.clock`, the instance `AppState.clock` and the resync loop
//! read). The tests advance it; the 30 s period is the design's compile-time
//! constant (D-2).
//!
//! Pending scenarios (their DELIVER step has not landed) are
//! `#[should_panic(expected = "<semantic RED>")]`: the body runs on every run
//! and fails at the recovery assertion until the pass exists. The `expected`
//! text names that assertion, so a body that fails for another reason
//! (fixture, fault that did not land) does not count as RED.
//!
//! Tier 1 (seeded `SimClock` advanced by the test) over the production
//! composition. Specification: `docs/feature/reconciler-convergence-guarantees/
//! distill/test-scenarios.md`.

#![allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::Stream;
use overdrive_control_plane::api::{AllocStateWire, AllocStatusResponse, SubmitWorkloadRequest};
use overdrive_control_plane::node_adapters::IntentStoreAdapter;
use overdrive_control_plane::{
    BootAdapters, ServerConfig, ServerHandle, run_server_with_obs_and_driver,
};
use overdrive_core::aggregate::{DriverInput, JobSpecInput, ResourcesInput, VmInput};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::guest_network::GuestNetworkExecWiring;
use overdrive_core::id::NodeId;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::DataplaneError;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::{
    IntentStore, IntentStoreError, IntentSubscriptionEvent, PutOutcome, StateSnapshot, TxnOp,
    TxnOutcome,
};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::view_store::SimViewStore;
use overdrive_sim::adapters::{SimDataplaneComposer, SimGuestDnsFactory};
use overdrive_testing::overdrive_test;
use tempfile::TempDir;
use tracing::Subscriber;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

use super::workload_lifecycle::wait::advance_and_settle;

/// The convergence tick cadence the injected clock is advanced by.
const TICK: Duration = Duration::from_millis(100);
/// The resync period: the design's compile-time constant (feature delta D-2).
const RESYNC_PERIOD: Duration = Duration::from_secs(30);
/// Queue time allowed on top of one period for a recovery: the pass submits
/// every managed target at once and admission is capped (feature delta, the
/// bound "one period plus queue time").
const QUEUE_ALLOWANCE: Duration = Duration::from_secs(5);
/// Injected time for a Running workload's handoffs (service lifecycle publishes
/// the backend row, the service-map hydrator programs the map) to complete.
const HANDOFF_ALLOWANCE: Duration = Duration::from_secs(10);
/// The hydrator's retry backoff (`RESTART_BACKOFF_DURATION`) plus tick slack:
/// a failed dataplane update is retried by this instant, well inside one
/// period, so the retry cannot be the pass.
const HYDRATOR_RETRY_BOUND: Duration = Duration::from_secs(10);

/// Ticks that carry the hydrator from a published backend row to its first
/// (failed) update, well inside its backoff.
const UPDATE_ATTEMPT_TICKS: usize = 3;

/// The event the action shim emits once per failed service-map update.
const UPDATE_FAILED_EVENT: &str = "dataplane.update_service.failed";

/// Appends the name of every tracing event this process emits.
#[derive(Clone, Default)]
struct EventNames(Arc<Mutex<Vec<String>>>);

impl<S> Layer<S> for EventNames
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        self.0.lock().expect("event names mutex").push(event.metadata().name().to_owned());
    }
}

/// How many `dataplane.update_service.failed` events this process has emitted.
/// The collector is the process's one global subscriber, installed on first
/// use (call it before boot): the failure is logged from spawned tasks on other
/// threads, which a thread-local default would miss. Each nextest test is its
/// own process, so the count is this test's.
fn failed_update_events() -> usize {
    static EVENTS: OnceLock<EventNames> = OnceLock::new();
    let names = EVENTS.get_or_init(|| {
        let names = EventNames::default();
        tracing::subscriber::set_global_default(tracing_subscriber::registry().with(names.clone()))
            .expect("the event collector is this test process's one global subscriber");
        names
    });
    names.0.lock().expect("event names mutex").iter().filter(|n| *n == UPDATE_FAILED_EVENT).count()
}

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

/// The node's intent store behind the `IntentStore` port: every call goes to
/// the real store, except that `get` fails while the fault is armed. The wrapper
/// is the intent-read fault (`IntentRead` hydrate error) delivered at the
/// existing driven port, the way `GatedIntentStore` interposes in the gateway
/// boot tests.
struct IntentReadFaultStore {
    inner: Arc<dyn IntentStore>,
    armed: Arc<AtomicBool>,
}

#[async_trait]
impl IntentStore for IntentReadFaultStore {
    async fn get(&self, key: &[u8]) -> Result<Option<Bytes>, IntentStoreError> {
        if self.armed.load(Ordering::SeqCst) {
            return Err(IntentStoreError::Io(std::io::Error::other(
                "scripted intent read failure",
            )));
        }
        self.inner.get(key).await
    }

    async fn put(&self, key: &[u8], value: &[u8]) -> Result<(), IntentStoreError> {
        self.inner.put(key, value).await
    }

    async fn put_if_absent(
        &self,
        key: &[u8],
        value: &[u8],
    ) -> Result<PutOutcome, IntentStoreError> {
        self.inner.put_if_absent(key, value).await
    }

    async fn delete(&self, key: &[u8]) -> Result<(), IntentStoreError> {
        self.inner.delete(key).await
    }

    async fn txn(&self, ops: Vec<TxnOp>) -> Result<TxnOutcome, IntentStoreError> {
        self.inner.txn(ops).await
    }

    async fn watch(
        &self,
        prefix: &[u8],
    ) -> Result<Box<dyn Stream<Item = IntentSubscriptionEvent> + Send + Unpin>, IntentStoreError>
    {
        self.inner.watch(prefix).await
    }

    async fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Bytes, Bytes)>, IntentStoreError> {
        self.inner.scan_prefix(prefix).await
    }

    async fn export_snapshot(&self) -> Result<StateSnapshot, IntentStoreError> {
        self.inner.export_snapshot().await
    }

    async fn bootstrap_from(&self, snapshot: StateSnapshot) -> Result<(), IntentStoreError> {
        self.inner.bootstrap_from(snapshot).await
    }
}

/// The driven-port handles a test keeps to inject its fault.
struct Ports {
    /// The observation store the node reads and writes. `None` boots over the
    /// host observation store the data directory holds.
    observation: Option<Arc<SimObservationStore>>,
    /// The reconciler View store. `None` boots over the host redb store.
    views: Option<Arc<SimViewStore>>,
    dataplane: Arc<SimDataplane>,
    /// While set, the intent store's `get` fails. `Some` boots the node over
    /// the fault-capable intent store wrapper.
    intent_read_fault: Option<Arc<AtomicBool>>,
}

impl Ports {
    fn new() -> Self {
        Self {
            observation: None,
            views: None,
            dataplane: Arc::new(SimDataplane::new()),
            intent_read_fault: None,
        }
    }

    fn with_intent_read_fault() -> Self {
        Self { intent_read_fault: Some(Arc::new(AtomicBool::new(false))), ..Self::new() }
    }

    fn with_observation() -> Self {
        Self {
            observation: Some(Arc::new(SimObservationStore::single_peer(
                NodeId::new("local").expect("node id"),
                0,
            ))),
            ..Self::new()
        }
    }
}

/// One lifetime of the node over a data directory that may outlive it.
struct Node {
    handle: ServerHandle,
    clock: Arc<SimClock>,
    /// The injected clock's reading when this lifetime of the node booted.
    booted_at: Duration,
    ports: Ports,
    client: reqwest::Client,
    base: String,
}

/// The two workload kinds a test deploys through the public API.
#[derive(Clone, Copy)]
enum Kind {
    /// A Job: Running is the whole outcome.
    Job,
    /// A Service with one TCP listener: Running and the service map programmed.
    Service,
}

/// The transient read fault that ends the first evaluation at its hydrate.
#[derive(Clone, Copy)]
enum HydrateFault {
    /// The observation store's allocation reads fail (`ObservationRead`).
    ObservationRead,
    /// The intent store's reads fail (`IntentRead`).
    IntentRead,
}

impl Node {
    async fn boot(dir: &TempDir, ports: Ports) -> Self {
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
        let mut boot: BootAdapters = super::node_adapters::boot_adapters(&config);
        boot.dataplane =
            Arc::new(SimDataplaneComposer::new(Arc::clone(&ports.dataplane), Ipv4Addr::LOCALHOST));
        boot.guest_dns = Arc::new(SimGuestDnsFactory::default());
        boot.clock = Arc::clone(&clock_port);
        if let Some(observation) = &ports.observation {
            boot.observation = Arc::clone(observation) as Arc<dyn ObservationStore>;
        }
        if let Some(views) = &ports.views {
            boot.stores.views = Arc::clone(views) as _;
        }
        if let Some(armed) = &ports.intent_read_fault {
            let intent: Arc<dyn IntentStore> = Arc::new(IntentReadFaultStore {
                inner: Arc::clone(boot.stores.intent.store()),
                armed: Arc::clone(armed),
            });
            boot.stores.intent =
                IntentStoreAdapter::new(intent, boot.stores.intent.path().to_path_buf());
        }
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
        let booted_at = clock.unix_now();
        Self { handle, clock, booted_at, ports, client, base }
    }

    /// Injected time since this lifetime of the node booted.
    fn elapsed(&self) -> Duration {
        self.clock.unix_now().saturating_sub(self.booted_at)
    }

    /// Advance the injected clock to exactly `at` since boot, in `TICK` steps.
    /// Never moves backwards.
    async fn step_to(&self, at: Duration) {
        self.step(at.saturating_sub(self.elapsed())).await;
    }

    async fn deploy(&self, kind: Kind, id: &str) {
        match kind {
            Kind::Job => self.deploy_job(id).await,
            Kind::Service => self.deploy_service(id).await,
        }
    }

    async fn deploy_job(&self, id: &str) {
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
        self.submit(spec, id).await;
    }

    async fn deploy_service(&self, id: &str) {
        let spec = SubmitSpecInput::Service(ServiceSpecInput {
            id: id.to_owned(),
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
        self.submit(spec, id).await;
    }

    async fn submit(&self, spec: SubmitSpecInput, id: &str) {
        let response = self
            .client
            .post(format!("{}/v1/workloads", self.base))
            .json(&SubmitWorkloadRequest { spec })
            .send()
            .await
            .expect("deploy reaches the public API");
        assert!(response.status().is_success(), "deploy {id}: {response:?}");
    }

    /// `overdrive workload describe`'s allocation read, through the public API.
    async fn allocs(&self, id: &str) -> AllocStatusResponse {
        let response = self
            .client
            .get(format!("{}/v1/allocs?job={id}", self.base))
            .send()
            .await
            .expect("GET /v1/allocs reaches the public API");
        assert!(response.status().is_success(), "GET /v1/allocs: {response:?}");
        response.json().await.expect("AllocStatusResponse body")
    }

    async fn is_running(&self, id: &str) -> bool {
        self.allocs(id).await.rows.iter().any(|row| row.state == AllocStateWire::Running)
    }

    /// Advance the injected clock by `span` in `TICK` steps.
    async fn step(&self, span: Duration) {
        let mut remaining = span;
        while !remaining.is_zero() {
            let step = remaining.min(TICK);
            advance_and_settle(&self.clock, step).await;
            remaining -= step;
        }
    }

    /// Advance until `id` is Running or `within` of injected time has passed.
    /// Returns whether it is Running.
    async fn step_until_running(&self, id: &str, within: Duration) -> bool {
        let mut elapsed = Duration::ZERO;
        loop {
            if self.is_running(id).await {
                return true;
            }
            if elapsed >= within {
                return false;
            }
            advance_and_settle(&self.clock, TICK).await;
            elapsed += TICK;
        }
    }

    /// The service map carries a backend: the end of the chain workload
    /// lifecycle, service lifecycle, service-map hydrator, dataplane.
    fn service_map_programmed(&self) -> bool {
        let dataplane = &self.ports.dataplane;
        dataplane.service_vip_keys().iter().any(|vip| {
            dataplane.service_backends(*vip).is_some_and(|backends| !backends.is_empty())
        })
    }

    /// Advance until the service map is programmed or `within` of injected time has passed.
    async fn step_until_service_map_programmed(&self, within: Duration) -> bool {
        let mut elapsed = Duration::ZERO;
        loop {
            if self.service_map_programmed() {
                return true;
            }
            if elapsed >= within {
                return false;
            }
            advance_and_settle(&self.clock, TICK).await;
            elapsed += TICK;
        }
    }

    fn obs(&self) -> &SimObservationStore {
        self.ports
            .observation
            .as_deref()
            .expect("this node was booted over the Sim observation store")
    }

    /// Take the node down with the injected clock frozen: no convergence tick
    /// or pass can fire during the shutdown.
    async fn shutdown_frozen(self) {
        let Self { handle, .. } = self;
        if let Err(error) = handle.shutdown(Duration::from_secs(2)).await {
            eprintln!("diagnostic=node_shutdown_error error={error}");
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

    /// A Job wedged by an `ObservationRead` hydrate error: the state only a
    /// later pass can resolve.
    async fn wedge_with_failed_first_hydrate(&self, id: &str) {
        self.wedge(Kind::Job, HydrateFault::ObservationRead, id).await;
    }

    fn set_hydrate_fault(&self, fault: HydrateFault, armed: bool) {
        match fault {
            HydrateFault::ObservationRead => {
                self.obs().script_alloc_status_rows_read_failure(armed);
            }
            HydrateFault::IntentRead => {
                self.ports
                    .intent_read_fault
                    .as_ref()
                    .expect("this node was booted over the fault-capable intent store")
                    .store(armed, Ordering::SeqCst);
            }
        }
    }

    /// Deploy `id` of `kind` while `fault` is armed, wait long enough for the
    /// first evaluation to hit it, and clear it. Leaves a target with intent,
    /// no allocation row and no pending evaluation.
    async fn wedge(&self, kind: Kind, fault: HydrateFault, id: &str) {
        self.set_hydrate_fault(fault, true);
        self.deploy(kind, id).await;
        self.step(Duration::from_secs(1)).await;
        // The public allocation read shares the failing store read, so the store
        // recovers before the precondition is read back.
        self.set_hydrate_fault(fault, false);
        assert!(
            self.allocs(id).await.rows.is_empty(),
            "precondition: the failed hydrate left {id} with no allocation row"
        );
    }
}

/// Outcome anchor: OUT-RCG-HYDRATE-ERROR-RECOVERS
/// Slice 01 AC1 (`ObservationRead`), AC4 — a Service whose first hydrate failed
/// has intent and no allocation row; the pass enumerates it from intent and
/// re-evaluates it, and the chain workload lifecycle, service lifecycle,
/// service-map hydrator carries it to a programmed service map.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given `payments` is a Service deployed while the observation store's reads
///   fail, and the first evaluation ended at its hydrate
///  And the store recovers
/// When one resync period passes with no second deploy
/// Then `payments` is Running
///  And the service map is programmed with its backend.
#[overdrive_test(guest = "cgroup-default")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_service_whose_first_hydrate_failed_is_running_and_in_the_service_map_by_the_next_pass() {
    let dir = TempDir::new().expect("tempdir");
    let node = Node::boot(&dir, Ports::with_observation()).await;

    node.wedge(Kind::Service, HydrateFault::ObservationRead, "payments").await;

    let running = node.step_until_running("payments", RESYNC_PERIOD + QUEUE_ALLOWANCE).await;
    assert!(
        running,
        "the service whose first hydrate failed was not Running within one resync period of \
         the store recovering: {:?}",
        node.allocs("payments").await
    );
    assert!(
        node.step_until_service_map_programmed(HANDOFF_ALLOWANCE).await,
        "the service map was not programmed after the recovered service reached Running"
    );
    node.shutdown().await;
}

/// Outcome anchor: OUT-RCG-INTENT-READ-ERROR-RECOVERS
/// Slice 01 AC1 (`IntentRead`) — an intent read that fails ends the first
/// evaluation at its hydrate with no requeue; the pass re-evaluates the target
/// once intent reads recover.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given `payments` is a Service deployed while the intent store's reads fail,
///   and the first evaluation ended at its hydrate
///  And the intent store recovers
/// When one resync period passes with no second deploy
/// Then `payments` is Running
///  And the service map is programmed with its backend.
#[overdrive_test(guest = "cgroup-default")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_service_whose_first_intent_read_failed_is_running_and_in_the_service_map_by_the_next_pass()
 {
    let dir = TempDir::new().expect("tempdir");
    let node = Node::boot(&dir, Ports::with_intent_read_fault()).await;

    node.wedge(Kind::Service, HydrateFault::IntentRead, "payments").await;

    let running = node.step_until_running("payments", RESYNC_PERIOD + QUEUE_ALLOWANCE).await;
    assert!(
        running,
        "the service whose first intent read failed was not Running within one resync period of \
         the intent store recovering: {:?}",
        node.allocs("payments").await
    );
    assert!(
        node.step_until_service_map_programmed(HANDOFF_ALLOWANCE).await,
        "the service map was not programmed after the recovered service reached Running"
    );
    node.shutdown().await;
}

/// Outcome anchor: OUT-RCG-VIEW-WRITE-ERROR-RECOVERS
/// Slice 01 AC2 — a failed View write ends the evaluation before dispatch, so
/// no action is dispatched on the failing tick; a later pass dispatches and the
/// service reaches a programmed service map.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given the reconciler View store fails its writes
///  And `payments` is a Service that is deployed
/// When the failing evaluations have run
/// Then no allocation has been started
/// When the View store recovers and one resync period passes
/// Then `payments` is Running
///  And the service map is programmed with its backend.
#[overdrive_test(guest = "cgroup-default")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_service_whose_view_write_failed_is_running_and_in_the_service_map_on_a_later_pass() {
    let dir = TempDir::new().expect("tempdir");
    let views = Arc::new(SimViewStore::new());
    let node = Node::boot(&dir, Ports { views: Some(Arc::clone(&views)), ..Ports::new() }).await;

    // GIVEN the View store fails its writes (after the boot probe) and the workload is deployed.
    views.inject_fsync_failure();
    node.deploy_service("payments").await;
    node.step(Duration::from_secs(2)).await;

    // THEN the failing tick dispatched nothing: no allocation was started.
    assert!(
        node.allocs("payments").await.rows.is_empty(),
        "precondition: the failed View write ended the evaluation before dispatch"
    );

    // WHEN the View store recovers and one period passes.
    views.clear_fsync_failure();
    let running = node.step_until_running("payments", RESYNC_PERIOD + QUEUE_ALLOWANCE).await;

    // THEN the workload is Running.
    assert!(
        running,
        "the service whose View write failed was not Running within one resync period of the \
         store recovering: {:?}",
        node.allocs("payments").await
    );
    assert!(
        node.step_until_service_map_programmed(HANDOFF_ALLOWANCE).await,
        "the service map was not programmed after the recovered service reached Running"
    );
    node.shutdown().await;
}

/// Outcome anchor: OUT-RCG-FAILED-DATAPLANE-RETRIES-AFTER-BACKOFF
/// Slice 01 AC3, feature delta D-3 — a failed dataplane update inside the
/// hydrator's backoff window is retried once the backoff elapses, with no
/// backend-row write and without waiting for the pass.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given `checkout` is a Service whose first service-map update fails
///  And the hydrator attempted that update (one failed-update event) and the next
///    tick fell inside its backoff
/// When the backoff elapses and no backend row is written
/// Then the service map is programmed with the Service's backend.
#[overdrive_test(guest = "cgroup-default")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_service_whose_dataplane_update_failed_is_retried_after_its_backoff_without_a_row_write()
{
    let dir = TempDir::new().expect("tempdir");
    // Install the event collector before boot so no failure event is missed.
    assert_eq!(failed_update_events(), 0, "precondition: no failed update before the deploy");
    let node = Node::boot(&dir, Ports::with_observation()).await;

    // GIVEN the first service-map update fails and the Service is deployed.
    node.ports.dataplane.script_update_service_failure(DataplaneError::LoadFailed(
        "scripted service-map update failure".to_owned(),
    ));
    node.deploy_service("checkout").await;

    // AND the hydrator attempted the update once the backend row existed: the
    // scripted failure left the service map unprogrammed, inside the backoff.
    let mut service_id = None;
    for _ in 0..300 {
        advance_and_settle(&node.clock, TICK).await;
        let backends = node.obs().all_service_backends_rows().await.expect("read backend rows");
        if let Some(row) = backends.first() {
            service_id = Some(row.service_id);
            break;
        }
    }
    let service_id = service_id.expect("precondition: the Service published a backend row");
    for _ in 0..UPDATE_ATTEMPT_TICKS {
        advance_and_settle(&node.clock, TICK).await;
    }
    assert_eq!(
        failed_update_events(),
        1,
        "precondition: the scripted failure was consumed by exactly one real update attempt"
    );
    assert!(
        node.ports.dataplane.service_vip_keys().is_empty(),
        "precondition: the failed update left the service map unprogrammed"
    );
    let rows_after_failure = node.obs().all_service_backends_rows().await.expect("read rows");

    // WHEN the backoff elapses (no pass is due for ~30 s).
    node.step(HYDRATOR_RETRY_BOUND).await;

    // THEN the service map is programmed, and no backend row was written to cause it.
    assert!(
        node.ports.dataplane.service_vip_keys().iter().any(|vip| node
            .ports
            .dataplane
            .service_backends(*vip)
            .is_some_and(|backends| !backends.is_empty())),
        "the service map was not programmed after the hydrator's backoff elapsed following a \
         failed update (service {service_id:?})"
    );
    assert_eq!(
        failed_update_events(),
        1,
        "the retry after the backoff must succeed: no further failed-update event"
    );
    let rows_after_retry = node.obs().all_service_backends_rows().await.expect("read rows");
    assert_eq!(
        rows_after_retry, rows_after_failure,
        "the retry must not have been caused by a backend-row write"
    );
    node.shutdown().await;
}

/// Outcome anchor: OUT-RCG-FRESH-DEPLOY-EVALUATED-AT-BOOT
/// Slice 01 AC4 — a deployed workload with intent and no allocation row is
/// enumerated from intent: the boot pass evaluates it.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given `payments` was deployed and its first evaluation failed, so the
///   intent exists and no allocation row does
///  And the node went down before any further pass
/// When the node restarts over the same intent
/// Then `payments` is Running well inside one period (the boot pass, not the periodic one).
#[overdrive_test(guest = "cgroup-default")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_node_restarted_over_a_deployed_intent_with_no_allocation_row_evaluates_it_at_boot() {
    let dir = TempDir::new().expect("tempdir");
    let first = Node::boot(&dir, Ports::with_observation()).await;
    first.wedge_with_failed_first_hydrate("payments").await;
    first.shutdown_frozen().await;

    let second = Node::boot(&dir, Ports::new()).await;
    assert!(
        second.allocs("payments").await.rows.is_empty(),
        "precondition: the restarted node starts with no allocation row for payments"
    );

    // The first periodic pass is one period away: only the boot pass can explain Running here.
    let running = second.step_until_running("payments", Duration::from_secs(10)).await;
    assert!(
        running,
        "the node restarted over a deployed intent with no allocation row did not evaluate it \
         at boot: {:?}",
        second.allocs("payments").await
    );
    second.shutdown().await;
}

/// Outcome anchor: OUT-RCG-ENUMERATION-FAILURE-ISOLATED
/// Slice 01 — one reconciler's enumeration failure never aborts the pass: the
/// pass logs it and continues, so every other reconciler's targets are still
/// submitted (named run `enumeration_failure_of_one_reconciler_does_not_starve_others`).
/// CONTRACT_SHAPE: bounded-change.
///
/// Given `payments` is wedged with intent and no allocation row
///  And every service-row read fails, so the `service-map-hydrator`'s
///   enumeration fails (it sorts before `workload-lifecycle` in the pass)
/// When one resync period passes
/// Then `payments` is Running: the pass continued past the failure.
#[overdrive_test(guest = "cgroup-default")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_enumeration_failure_of_one_reconciler_does_not_starve_the_others_in_the_pass() {
    let dir = TempDir::new().expect("tempdir");
    let node = Node::boot(&dir, Ports::with_observation()).await;

    node.wedge_with_failed_first_hydrate("payments").await;
    node.obs().script_service_rows_read_failure(true);

    let running = node.step_until_running("payments", RESYNC_PERIOD + QUEUE_ALLOWANCE).await;
    assert!(
        running,
        "the pass did not reach workload-lifecycle after another reconciler's enumeration \
         failed: {:?}",
        node.allocs("payments").await
    );
    node.obs().script_service_rows_read_failure(false);
    node.shutdown().await;
}

/// Outcome anchor: OUT-RCG-PASS-CADENCE
/// WK-3 — the loop runs a pass at boot, then at `previous + period` with no
/// drift, and at most one pass per period. The pass is observed through its
/// effect: a wedged workload (intent, no row) is only ever recovered by a pass.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given a node booted at t=0 (boot pass at t=0, next at one period)
///  And a workload wedged at t=2 s
/// Then it is not Running at period minus 2 s (no pass in between)
///  And it is Running at period plus 2 s (the pass at one period)
/// Given the clock then jumps past the second boundary in one step (a late pass)
///  And another workload is wedged after that pass
/// Then it is not Running two seconds before the third boundary
///  And it is Running one second after it: the next pass stayed anchored at
///   previous + period (a drifting loop would run it five seconds later).
#[overdrive_test(guest = "cgroup-default")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_resync_pass_stays_anchored_one_period_apart_with_no_drift_and_at_most_one_per_period()
{
    let dir = TempDir::new().expect("tempdir");
    let node = Node::boot(&dir, Ports::with_observation()).await;
    let period = RESYNC_PERIOD;

    // t = 2 s: wedge the first workload (cleared by t = 3 s).
    node.step(Duration::from_secs(2)).await;
    node.wedge_with_failed_first_hydrate("first").await;

    // t = period - 2 s: no pass since boot, so still wedged.
    node.step(Duration::from_secs(period.as_secs() - 5)).await;
    assert!(
        !node.is_running("first").await,
        "a pass ran before the first period elapsed: at most one pass per period"
    );
    // t = period + 2 s: the pass at one period recovered it.
    node.step(Duration::from_secs(4)).await;
    assert!(
        node.is_running("first").await,
        "the pass at the end of the first period did not recover the wedged workload"
    );

    // t = period + 3 s: wedge the second workload, then jump past the second boundary in ONE
    // clock step, so the pass due at 2 x period runs late (at 2 x period + 5 s).
    node.step(Duration::from_secs(1)).await;
    node.wedge_with_failed_first_hydrate("second").await;
    let jump = Duration::from_secs(period.as_secs() + 1);
    advance_and_settle(&node.clock, jump).await;
    assert!(
        node.step_until_running("second", Duration::from_secs(5)).await,
        "the late pass due at the second boundary did not recover the second wedged workload"
    );

    // t = 2 x period + 6 s: wedge the third workload. The next pass is anchored at 3 x period.
    node.step(Duration::from_secs(1)).await;
    node.wedge_with_failed_first_hydrate("third").await;
    node.step(Duration::from_secs(period.as_secs() - 9)).await; // t = 3 x period - 2 s
    assert!(
        !node.is_running("third").await,
        "a pass ran before the third boundary: at most one pass per period"
    );
    node.step(Duration::from_secs(3)).await; // t = 3 x period + 1 s
    assert!(
        node.is_running("third").await,
        "the pass after a late pass drifted: it must run at previous + period, not at the late \
         pass's instant + period"
    );
    node.shutdown().await;
}

/// Outcome anchor: OUT-RCG-PASS-CATCH-UP
/// WK-3, catch-up — when the clock jumps over several periods at once the
/// missed passes run (anchored at `previous + period`, one per loop iteration)
/// and the schedule then stays on the boot grid: a pass runs at each multiple of
/// the period since boot, and no pass runs between multiples. The pass is
/// observed through its effect, a wedged workload only a pass recovers.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given a node booted at t=0 and a workload wedged at t=2 s
/// When the clock jumps to three periods plus five seconds in one step
/// Then the wedged workload is Running within five seconds: a pass ran
/// Given a second workload wedged at three periods plus fifteen seconds
/// Then it is not Running two seconds before the fourth boundary: no extra pass ran
///  And it is Running two seconds after it: the schedule stayed on the grid (a
///   loop that re-anchored at the jump would run it five seconds later).
#[overdrive_test(guest = "cgroup-default")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_clock_jump_over_several_periods_runs_the_missed_passes_and_stays_on_the_boot_grid() {
    let dir = TempDir::new().expect("tempdir");
    let node = Node::boot(&dir, Ports::with_observation()).await;
    let period = RESYNC_PERIOD;

    node.step_to(Duration::from_secs(2)).await;
    node.wedge_with_failed_first_hydrate("first").await;

    // One clock step over three boundaries (30 s, 60 s, 90 s).
    let jump_to = period * 3 + Duration::from_secs(5);
    advance_and_settle(&node.clock, jump_to.saturating_sub(node.elapsed())).await;
    assert!(
        node.step_until_running("first", Duration::from_secs(5)).await,
        "the pass due inside a clock jump over several periods did not recover the wedged \
         workload"
    );

    // The missed passes run one per loop iteration; let them finish, then wedge the second.
    node.step_to(period * 3 + Duration::from_secs(15)).await;
    node.wedge_with_failed_first_hydrate("second").await;

    node.step_to((period * 4).saturating_sub(Duration::from_secs(2))).await;
    assert!(
        !node.is_running("second").await,
        "a pass ran between two boundaries of the boot grid: at most one pass per period"
    );
    node.step_to(period * 4 + Duration::from_secs(2)).await;
    assert!(
        node.is_running("second").await,
        "the pass after a clock jump over several periods left the boot grid: it must run at \
         the next multiple of the period since boot"
    );
    node.shutdown().await;
}
