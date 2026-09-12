//! Issue-283 regression: the actual broker/convergence owner under slow Driver effects.
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
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, JobSpecInput, ResourcesInput, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole, ProbeStatus};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, ExitEvent,
    ExitKind, Resources,
};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore, ObservationStoreError,
};
use overdrive_sim::adapters::{
    SimKek, clock::SimClock, dataplane::SimDataplane, driver::SimDriver,
    observation_store::SimObservationStore,
};
use parking_lot::Mutex;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, OnceLock};
use std::time::Duration;
use tokio::sync::{Notify, Semaphore, mpsc};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

static SERVER_SCENARIO_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Debug)]
struct CapturedEvent {
    name: String,
    metadata_target: String,
    fields: BTreeMap<String, String>,
}

impl CapturedEvent {
    fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }

    fn u64_field(&self, name: &str) -> Option<u64> {
        self.field(name)?.trim_matches('"').parse().ok()
    }
}

#[derive(Default)]
struct DrainEventGate {
    reached: AtomicBool,
    reached_notify: Notify,
    released: std::sync::Mutex<bool>,
    released_notify: Condvar,
}

impl DrainEventGate {
    fn wait_in_layer(&self) {
        self.reached.store(true, Ordering::SeqCst);
        self.reached_notify.notify_waiters();
        let mut released = self.released.lock().expect("drain gate mutex");
        while !*released {
            released = self.released_notify.wait(released).expect("drain gate wait");
        }
        drop(released);
    }

    async fn wait_reached(&self) {
        while !self.reached.load(Ordering::SeqCst) {
            self.reached_notify.notified().await;
        }
    }

    fn release(&self) {
        *self.released.lock().expect("drain gate mutex") = true;
        self.released_notify.notify_all();
    }
}

#[derive(Clone, Default)]
struct TraceState {
    events: Arc<std::sync::Mutex<Vec<CapturedEvent>>>,
    drain_gate: Arc<std::sync::Mutex<Option<Arc<DrainEventGate>>>>,
}

#[derive(Clone)]
struct CaptureLayer(TraceState);

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

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
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
        let name = event.metadata().name().to_owned();
        self.0.events.lock().expect("trace events mutex").push(CapturedEvent {
            name: name.clone(),
            metadata_target: event.metadata().target().to_owned(),
            fields: visitor.0,
        });
        if name == "convergence.drain.completed"
            && let Some(gate) = self.0.drain_gate.lock().expect("drain gate mutex").take()
        {
            gate.wait_in_layer();
        }
    }
}

fn trace_state() -> &'static TraceState {
    static TRACE: OnceLock<TraceState> = OnceLock::new();
    TRACE.get_or_init(|| {
        let state = TraceState::default();
        tracing::subscriber::set_global_default(
            Registry::default().with(CaptureLayer(state.clone())),
        )
        .expect("vm-lifecycle test binary installs one global trace subscriber");
        state
    })
}

fn trace_cursor() -> usize {
    trace_state().events.lock().expect("trace events mutex").len()
}

fn trace_since(cursor: usize) -> Vec<CapturedEvent> {
    trace_state().events.lock().expect("trace events mutex")[cursor..].to_vec()
}

fn event_target(event: &CapturedEvent) -> Option<&str> {
    event.field("target").map(|value| value.trim_matches('"'))
}

fn evaluation_keys(events: &[CapturedEvent], name: &str) -> BTreeSet<(String, String, u64)> {
    events
        .iter()
        .filter(|event| event.name == name)
        .filter_map(|event| {
            Some((
                event.field("reconciler")?.trim_matches('"').to_owned(),
                event_target(event)?.to_owned(),
                event.u64_field("tick")?,
            ))
        })
        .collect()
}

fn drain_events(events: &[CapturedEvent]) -> Vec<&CapturedEvent> {
    events.iter().filter(|event| event.name == "convergence.drain.completed").collect()
}

struct ExitEmissionGate {
    alloc: AllocationId,
    entered: Notify,
    entered_flag: AtomicBool,
    release: Semaphore,
}

impl ExitEmissionGate {
    async fn wait_entered(&self) {
        while !self.entered_flag.load(Ordering::SeqCst) {
            self.entered.notified().await;
        }
    }
}

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
    release: Semaphore,
    events: Mutex<Vec<DriverEvent>>,
    exit_emission_gates: Vec<Arc<ExitEmissionGate>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DriverEvent {
    Entered(Effect, AllocationId),
    Returned(Effect, AllocationId),
}

#[async_trait]
impl Driver for DelayedDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Exec
    }
    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        self.events.lock().push(DriverEvent::Entered(Effect::Start, spec.alloc.clone()));
        if self.effect == Effect::Start && spec.alloc.as_str().starts_with("alloc-slow") {
            self.entered.send(Effect::Start).unwrap();
            self.release.acquire().await.unwrap().forget();
        }
        let result = self.inner.start(spec).await?;
        self.events.lock().push(DriverEvent::Returned(Effect::Start, spec.alloc.clone()));
        Ok(result)
    }
    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        self.events.lock().push(DriverEvent::Entered(Effect::Stop, handle.alloc.clone()));
        self.inner.stop(handle).await?;
        if self.effect == Effect::Stop && handle.alloc.as_str().starts_with("alloc-slow") {
            self.entered.send(Effect::Stop).unwrap();
            self.release.acquire().await.unwrap().forget();
        }
        self.events.lock().push(DriverEvent::Returned(Effect::Stop, handle.alloc.clone()));
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
        if let Some(gate) = self.exit_emission_gates.iter().find(|gate| gate.alloc == handle.alloc)
            && gate
                .entered_flag
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            gate.entered.notify_waiters();
            gate.release.acquire().await.unwrap().forget();
        }
        self.inner.release_for_exit_emission(handle).await;
    }
    fn take_exit_receiver(&self) -> Option<mpsc::Receiver<ExitEvent>> {
        self.inner.take_exit_receiver()
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

async fn submit_service(client: &reqwest::Client, base: &str, id: &str) {
    let body = SubmitWorkloadRequest {
        spec: SubmitSpecInput::Service(ServiceSpecInput {
            id: id.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            driver: DriverInput::Exec(ExecInput {
                command: "/bin/sleep".to_owned(),
                args: vec!["3600".to_owned()],
            }),
            listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
            startup_probes: vec![ProbeDescriptor {
                idx: ProbeIdx::new(0),
                role: ProbeRole::Startup,
                mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_owned(), port: 8080 },
                timeout_seconds: 1,
                interval_seconds: 1,
                max_attempts: 1,
                failure_threshold: None,
                success_threshold: None,
                inferred: false,
            }],
            readiness_probes: Vec::new(),
            liveness_probes: Vec::new(),
        }),
    };
    let response = client.post(format!("{base}/v1/workloads")).json(&body).send().await.unwrap();
    assert!(response.status().is_success(), "submit Service {id}: {response:?}");
    let _: SubmitWorkloadResponse = response.json().await.unwrap();
}

async fn running(obs: &SimObservationStore, id: &str) -> bool {
    obs.alloc_status_rows()
        .await
        .unwrap()
        .iter()
        .any(|row| row.workload_id.as_str() == id && row.state == AllocState::Running)
}

fn effect_targets(
    driver: &DelayedDriver,
    effect: Effect,
    returned: bool,
) -> BTreeSet<AllocationId> {
    driver
        .events
        .lock()
        .iter()
        .filter_map(|event| match event {
            DriverEvent::Returned(kind, alloc) if returned && *kind == effect => {
                Some(alloc.clone())
            }
            DriverEvent::Entered(kind, alloc) if !returned && *kind == effect => {
                Some(alloc.clone())
            }
            _ => None,
        })
        .collect()
}

async fn result_state_targets(obs: &SimObservationStore, effect: Effect) -> BTreeSet<AllocationId> {
    let state = if effect == Effect::Start { AllocState::Running } else { AllocState::Terminated };
    obs.alloc_status_rows()
        .await
        .unwrap()
        .into_iter()
        .filter(|row| row.workload_id.as_str().starts_with("slow") && row.state == state)
        .map(|row| row.alloc_id)
        .collect()
}

async fn workload_targets_for_allocations(
    obs: &SimObservationStore,
    allocations: &BTreeSet<AllocationId>,
) -> BTreeSet<String> {
    obs.alloc_status_rows()
        .await
        .unwrap()
        .into_iter()
        .filter(|row| allocations.contains(&row.alloc_id))
        .map(|row| format!("workload/{}", row.workload_id))
        .collect()
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
    let _serial = SERVER_SCENARIO_LOCK.lock().await;
    let _ = trace_state();
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
        release: Semaphore::new(0),
        events: Mutex::new(Vec::new()),
        exit_emission_gates: Vec::new(),
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
    driver.release.add_permits(1);
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
        "RED scaffold (S-VLL-01): seed={seed}: independent workload cannot progress while {effect:?} holds the production convergence owner"
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

/// CONTRACT_SHAPE: bounded-change.
/// Seed 283001 drives the real spawned convergence owner through a failed
/// `IssueSvid`, its one immediate confirmation, and the deferred no-action
/// requeue. The held SimClock remains before that retry deadline, so a third
/// admission must not occur until the wall-clock boundary is reached.
#[tokio::test]
async fn convergence_owner_defers_no_action_retry_before_deadline() {
    let _serial = SERVER_SCENARIO_LOCK.lock().await;
    let seed = 283_001;
    eprintln!("vm-lifecycle retry eligibility seed={seed}");
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(SimClock::new());
    let node = NodeId::new("local").unwrap();
    let obs = Arc::new(SimObservationStore::single_peer(node.clone(), seed));
    let driver = Arc::new(SimDriver::with_clock(DriverType::Exec, clock.clone()));
    let config_dir = directory.path().join("operator");
    let config = ServerConfig {
        data_dir: directory.path().join("data"),
        operator_config_dir: config_dir.clone(),
        clock: clock.clone(),
        dataplane: Some(DataplaneConfig { client_iface: "lo".into(), backend_iface: "lo".into() }),
        dataplane_override: Some(Arc::new(SimDataplane::new())),
        ..ServerConfig::new(Arc::new(SimKek::for_boot()))
    };
    let server = run_server_with_obs_and_driver(config, obs.clone(), driver).await.unwrap();
    let base = format!("https://localhost:{}", server.local_addr().await.unwrap().port());
    let http = client(&config_dir);
    submit(&http, &base, "payments").await;
    for _ in 0..30 {
        advance(&clock, seed).await;
        if running(&obs, "payments").await {
            break;
        }
    }
    assert!(running(&obs, "payments").await, "seed={seed}: initial workload must run");
    // Let the startup SVID evaluation settle before injecting the one audit
    // failure used by this owner-path regression.
    for _ in 0..10 {
        advance(&clock, seed).await;
    }
    let trace_before = trace_cursor();

    let workload_id = WorkloadId::new("payments").unwrap();
    let alloc_id = AllocationId::new("alloc-payments-retry").unwrap();
    obs.write_alloc_lifecycle(
        AllocStatusRow {
            alloc_id: alloc_id.clone(),
            workload_id: workload_id.clone(),
            node_id: node.clone(),
            state: AllocState::Running,
            updated_at: LogicalTimestamp { counter: 10_000, writer: node.clone() },
            reason: None,
            detail: None,
            terminal: None,
            stderr_tail: None,
            kind: WorkloadKind::Job,
            listeners: Vec::new(),
            started_at: None,
            workload_addr: None,
            last_terminated: None,
            restart_count: 0,
        },
        overdrive_core::traits::observation_store::TransitionSource::Reconciler,
    )
    .await
    .unwrap();
    obs.inject_write_failure(ObservationStoreError::Unreachable { peer: "retry-audit".into() });

    // Five 103ms logical advances remain before the one-second retry boundary.
    for _ in 0..5 {
        advance(&clock, seed).await;
    }
    let before_deadline = trace_since(trace_before);
    let admissions_before = before_deadline
        .iter()
        .filter(|event| {
            event.name == "convergence.evaluation.admitted"
                && event.field("reconciler") == Some("svid-lifecycle")
                && event_target(event) == Some("workload/payments")
        })
        .count();
    assert_eq!(
        admissions_before, 2,
        "seed={seed}: failed issue plus one confirmation, never a third admission before deadline"
    );

    // Cross the injected wall-clock deadline and prove the deferred key is
    // eventually admitted by the same owner.
    for _ in 0..7 {
        advance(&clock, seed).await;
    }
    let after_deadline = trace_since(trace_before);
    let admissions_after = after_deadline
        .iter()
        .filter(|event| {
            event.name == "convergence.evaluation.admitted"
                && event.field("reconciler") == Some("svid-lifecycle")
                && event_target(event) == Some("workload/payments")
        })
        .count();
    assert!(admissions_after >= 3, "seed={seed}: retry must admit at/after its deadline");
    server.shutdown(Duration::from_secs(1)).await.unwrap();
}

/// Drive the existing production owner with distinct public workload targets.
/// The semaphore substitutes only external Driver latency; admission, pending
/// coalescing, storage, HTTP owners, and shutdown are the real composition.
async fn capacity_case(effect: Effect, held: usize, close_admission: bool) {
    let _serial = SERVER_SCENARIO_LOCK.lock().await;
    let _ = trace_state();
    let seed = 283_001;
    eprintln!("vm-lifecycle seed={seed} effect={effect:?} held={held} close={close_admission}");
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(SimClock::new());
    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("local").unwrap(), seed));
    let (entered, mut entries) = mpsc::unbounded_channel();
    let driver = Arc::new(DelayedDriver {
        inner: SimDriver::with_clock(DriverType::Exec, clock.clone()),
        effect,
        entered,
        release: Semaphore::new(0),
        events: Mutex::new(Vec::new()),
        exit_emission_gates: Vec::new(),
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
    let base = format!("https://localhost:{}", server.local_addr().await.unwrap().port());
    let client = client(&config_dir);
    let trace_before_effect = (effect == Effect::Start).then(trace_cursor);
    for index in 0..held {
        submit(&client, &base, &format!("slow{index}")).await;
    }
    if effect == Effect::Stop {
        for _ in 0..100 {
            advance(&clock, seed).await;
        }
        for index in 0..held {
            let id = format!("slow{index}");
            assert!(running(&obs, &id).await, "seed={seed}: healthy setup failed for {id}");
            let response =
                client.post(format!("{base}/v1/workloads/{id}/stop")).send().await.unwrap();
            assert!(response.status().is_success(), "seed={seed}: stop rejected for {id}");
        }
    }
    let effect_trace_cursor = trace_before_effect.unwrap_or_else(trace_cursor);
    let mut entered_while_held = 0;
    for _ in 0..100 {
        advance(&clock, seed).await;
        while let Ok(observed) = entries.try_recv() {
            assert_eq!(observed, effect, "seed={seed}: wrong held effect");
            entered_while_held += 1;
        }
        if entered_while_held == held {
            break;
        }
    }
    submit(&client, &base, "independent").await;
    let mut while_held = false;
    for _ in 0..50 {
        advance(&clock, seed).await;
        while_held |= running(&obs, "independent").await;
    }
    let after_release;
    if close_admission {
        let entered_targets_at_close = effect_targets(&driver, effect, false);
        assert_eq!(
            entered_targets_at_close.len(),
            entered_while_held,
            "seed={seed}: entry ledger disagrees"
        );
        // Release one real Driver call at a time. Each returned allocation must
        // reach the shim-authored post-result row while shutdown still waits for
        // every remaining hold. A port return alone is not result consumption.
        let shutdown = tokio::spawn(server.shutdown(Duration::from_secs(1)));
        for _ in 0..10 {
            advance(&clock, seed).await;
        }
        let mut held_waits = Vec::new();
        let mut stages = Vec::new();
        for released in 1..=entered_while_held {
            held_waits.push(!shutdown.is_finished());
            driver.release.add_permits(1);
            for _ in 0..100 {
                advance(&clock, seed).await;
                let returned = effect_targets(&driver, effect, true);
                let owner_completed = evaluation_keys(
                    &trace_since(effect_trace_cursor),
                    "convergence.evaluation.completed",
                );
                let completed_targets: BTreeSet<_> =
                    owner_completed.iter().map(|(_, target, _)| target.clone()).collect();
                if returned.len() == released
                    && result_state_targets(&obs, effect).await == returned
                    && (entered_while_held != held
                        || completed_targets
                            == workload_targets_for_allocations(&obs, &returned).await)
                {
                    break;
                }
            }
            stages.push((
                effect_targets(&driver, effect, true),
                result_state_targets(&obs, effect).await,
            ));
            if entered_while_held == held && released < held {
                assert!(
                    !shutdown.is_finished()
                        && drain_events(&trace_since(effect_trace_cursor)).is_empty(),
                    "RED scaffold (S-VLL-06a staged drain): seed={seed}: shutdown/report preceded the final held result"
                );
            }
        }
        // The current serial owner may have pre-drained additional evaluations
        // before the first held Driver call. Release those during cleanup too;
        // their later returns do not establish eight concurrent active effects.
        driver.release.add_permits(held);
        tokio::time::timeout(Duration::from_secs(10), shutdown).await.unwrap().unwrap().unwrap();
        after_release = running(&obs, "independent").await;
        assert!(
            held_waits.iter().all(|waiting| *waiting),
            "seed={seed}: a held result was discarded"
        );
        for (index, (returned, states)) in stages.iter().enumerate() {
            assert_eq!(returned.len(), index + 1, "seed={seed}: missing Driver result at {index}");
            assert!(
                returned.is_subset(&entered_targets_at_close),
                "seed={seed}: result without pre-close Driver entry"
            );
            assert_eq!(states, returned, "seed={seed}: shim did not publish every returned result");
        }
        let returned_at_join = effect_targets(&driver, effect, true);
        let entered_at_join = effect_targets(&driver, effect, false);
        assert_eq!(
            returned_at_join, entered_at_join,
            "seed={seed}: every entered Driver call returned"
        );
        assert_eq!(
            result_state_targets(&obs, effect).await,
            returned_at_join,
            "seed={seed}: final shim rows"
        );
        assert!(
            driver.events.lock().iter().all(|event| !matches!(
                event,
                DriverEvent::Entered(_, alloc) if alloc.as_str().starts_with("alloc-independent")
            )),
            "seed={seed}: ninth workload reached the Driver boundary"
        );
        assert!(
            obs.alloc_status_rows()
                .await
                .unwrap()
                .iter()
                .all(|row| row.workload_id.as_str() != "independent"),
            "seed={seed}: ninth workload acquired an allocation observation"
        );
        eprintln!(
            "vm-lifecycle seed={seed} effect={effect:?} entered_before_close={} returned_and_published_at_join={} sequential_release_stages={} ninth_driver_entries=0 ninth_allocation_rows=0; complete owner oracle follows the eight-way RED precondition",
            entered_targets_at_close.len(),
            returned_at_join.len(),
            stages.len()
        );
        // Current production reaches only one sequential-release stage, then
        // fails THIS concurrent-entry precondition. Later serial batch returns
        // cannot establish the eight-active or private owner-consumption oracle.
        assert_eq!(
            entered_while_held, held,
            "RED scaffold (S-VLL-06a admission): seed={seed} all eight slots must admit"
        );
        let owner_events = trace_since(effect_trace_cursor);
        let admissions = evaluation_keys(&owner_events, "convergence.evaluation.admitted");
        let completions = evaluation_keys(&owner_events, "convergence.evaluation.completed");
        let expected_targets: BTreeSet<_> =
            (0..held).map(|index| format!("workload/slow{index}")).collect();
        let expected_allocations: BTreeSet<_> = (0..held)
            .map(|index| AllocationId::new(&format!("alloc-slow{index}-0")).unwrap())
            .collect();
        let admission_targets: BTreeSet<_> =
            admissions.iter().map(|(_, target, _)| target.clone()).collect();
        assert_eq!(
            admissions.len(),
            held,
            "RED scaffold (S-VLL-06a owner admission): seed={seed}: one admission per held target"
        );
        assert_eq!(admission_targets, expected_targets, "seed={seed}: admitted target ledger");
        assert_eq!(
            entered_targets_at_close, expected_allocations,
            "seed={seed}: every admission matches one held Driver entry"
        );
        assert_eq!(
            owner_events
                .iter()
                .filter(|event| event.name == "convergence.evaluation.admitted")
                .count(),
            held,
            "seed={seed}: duplicate admission event"
        );
        assert_eq!(
            owner_events
                .iter()
                .filter(|event| event.name == "convergence.evaluation.completed")
                .count(),
            held,
            "seed={seed}: duplicate completion event"
        );
        assert!(
            owner_events
                .iter()
                .filter(|event| event.name == "convergence.evaluation.completed")
                .all(|event| event.u64_field("elapsed_ms").is_some()
                    && event.field("outcome").is_some()),
            "seed={seed}: completion event fields"
        );
        assert_eq!(
            completions, admissions,
            "RED scaffold (S-VLL-06a owner consumption): seed={seed}: every admitted target/tick must be consumed"
        );
        let admitted_active: BTreeSet<_> = owner_events
            .iter()
            .filter(|event| event.name == "convergence.evaluation.admitted")
            .filter_map(|event| event.u64_field("active"))
            .collect();
        assert_eq!(admitted_active, (1..=held as u64).collect(), "seed={seed}: active ledger");
        assert!(
            owner_events
                .iter()
                .filter(|event| event.name == "convergence.evaluation.admitted")
                .all(|event| event.u64_field("capacity") == Some(8)
                    && event.u64_field("queue_ms").is_some()),
            "seed={seed}: admission event fields"
        );
        let drains = drain_events(&owner_events);
        assert_eq!(
            drains.len(),
            1,
            "RED scaffold (S-VLL-06a drain report): seed={seed}: sole owner drain event"
        );
        let drain = drains[0];
        assert!(drain.u64_field("elapsed_ms").is_some(), "seed={seed}: drain elapsed field");
        assert_eq!(drain.u64_field("admitted_at_close"), Some(8), "seed={seed}");
        assert_eq!(drain.u64_field("completed_during_drain"), Some(8), "seed={seed}");
        let drain_index = owner_events
            .iter()
            .position(|event| event.name == "convergence.drain.completed")
            .expect("sole drain event exists");
        assert!(
            owner_events
                .iter()
                .enumerate()
                .filter(|(_, event)| event.name == "convergence.evaluation.completed")
                .all(|(index, _)| index < drain_index),
            "seed={seed}: all completions precede the drain report"
        );
        assert!(
            owner_events.iter().all(|event| {
                event_target(event) != Some("workload/independent")
                    || (event.name != "convergence.evaluation.admitted"
                        && event.name != "convergence.evaluation.completed")
            }),
            "seed={seed}: ninth evaluation was admitted or completed"
        );
        assert_eq!(
            entered_at_join, entered_targets_at_close,
            "seed={seed}: Driver entry set grew during drain"
        );
    } else {
        // One completion frees exactly one slot; it is enough for an eligible
        // independent target even when the other seven effects remain held.
        driver.release.add_permits(1);
        let mut progressed = while_held;
        for _ in 0..100 {
            advance(&clock, seed).await;
            progressed |= running(&obs, "independent").await;
            if progressed {
                break;
            }
        }
        after_release = progressed;
        driver.release.add_permits(held);
        for _ in 0..100 {
            advance(&clock, seed).await;
        }
        server.shutdown(Duration::from_secs(1)).await.unwrap();
    }
    eprintln!(
        "vm-lifecycle seed={seed} held={held} entered_while_held={entered_while_held} while_held={while_held} after_release={after_release} close={close_admission} shutdown=joined"
    );
    // The marker identifies a live, reproduced missing admission behavior. A
    // setup, transport, or cleanup panic cannot satisfy this expected RED.
    assert_eq!(
        entered_while_held, held,
        "RED scaffold (S-VLL-02/03): seed={seed} all requested slots must admit"
    );
    assert_eq!(while_held, held < 8, "RED scaffold (S-VLL-02/03): seed={seed} eight-slot boundary");
    assert_eq!(
        after_release, !close_admission,
        "RED scaffold (S-VLL-03/06): seed={seed} freed slot versus closed admission"
    );
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-02: seven pending starts leave capacity for independent convergence.
#[tokio::test]
async fn seven_held_starts_leave_one_progress_slot() {
    capacity_case(Effect::Start, 7, false).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-02: seven pending stops leave capacity for independent convergence.
#[tokio::test]
async fn seven_held_stops_leave_one_progress_slot() {
    capacity_case(Effect::Stop, 7, false).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-03: at eight starts the ninth waits, then one completion refills it.
#[tokio::test]
async fn eight_held_starts_bound_and_refill_admission() {
    capacity_case(Effect::Start, 8, false).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-03: at eight stops the ninth waits, then one completion refills it.
#[tokio::test]
async fn eight_held_stops_bound_and_refill_admission() {
    capacity_case(Effect::Stop, 8, false).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-06: shutdown drains eight starts and leaves the ninth unexecuted.
/// Match every admitted target/tick to its Driver return, shim row and owner
/// completion before the sole drain event; the ninth crosses no boundary.
#[tokio::test]
async fn admission_close_drains_owned_starts_without_admitting_ninth() {
    capacity_case(Effect::Start, 8, true).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-06: shutdown drains eight stops and leaves the ninth unexecuted.
/// Match every admitted target/tick to its Driver return, shim row and owner
/// completion before the sole drain event; the ninth crosses no boundary.
#[tokio::test]
async fn admission_close_drains_owned_stops_without_admitting_ninth() {
    capacity_case(Effect::Stop, 8, true).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-13: the composed runtime must publish no effect when its View write
/// fails. Drive valid desired-generation inputs, never fabricate a next View;
/// prove the same inputs dispatch after recovery. Seed 283001 is a preservation
/// witness, not a newly alleged production defect.
#[tokio::test]
async fn view_fsync_failure_prevents_dispatch_and_recovers() {
    use overdrive_control_plane::AppState;
    use overdrive_control_plane::identity_mgr::IdentityMgr;
    use overdrive_control_plane::reconciler_runtime::{
        ConvergenceError, ReconcilerRuntime, run_convergence_tick,
    };
    use overdrive_control_plane::view_store::ViewStoreExt;
    use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
    use overdrive_core::traits::clock::Clock;
    use overdrive_core::traits::intent_store::IntentStore;
    use overdrive_reconcilers::WorkloadLifecycleView;
    use overdrive_sim::adapters::{ca::SimCa, entropy::SimEntropy, view_store::SimViewStore};
    use overdrive_store_local::LocalIntentStore;

    let seed = 283_001;
    eprintln!("vm-lifecycle S-VLL-13 seed={seed} healthy/fsync-failure/recovery");
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(SimClock::new());
    let views = Arc::new(SimViewStore::new());
    let mut runtime = ReconcilerRuntime::new(directory.path(), views.clone()).unwrap();
    runtime.register(overdrive_control_plane::workload_lifecycle()).await.unwrap();
    let path = directory.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&path).unwrap());
    let node = NodeId::new("local").unwrap();
    let obs = Arc::new(SimObservationStore::single_peer(node.clone(), seed));
    let driver = Arc::new(SimDriver::with_clock(DriverType::Exec, clock.clone()));
    let allocator =
        overdrive_control_plane::test_default_allocator(store.clone() as Arc<dyn IntentStore>);
    let state = AppState::new(
        store,
        path,
        obs.clone(),
        Arc::new(runtime),
        driver.clone(),
        clock.clone(),
        Arc::new(SimDataplane::new()),
        Arc::new(SimCa::new(Arc::new(SimEntropy::new(seed)))),
        Arc::new(IdentityMgr::new(None)),
        node,
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    );
    let name = ReconcilerName::new("workload-lifecycle").unwrap();
    let control = TargetResource::new("workload/control").unwrap();
    let subject = TargetResource::new("workload/fsync-subject").unwrap();
    runtime_job_input(&state, "control").await;
    runtime_job_input(&state, "fsync-subject").await;
    let now = clock.now();
    let deadline = now + Duration::from_secs(1);
    run_convergence_tick(&state, &name, &control, now, 0, deadline).await.unwrap();
    assert!(running(&obs, "control").await, "seed={seed}: healthy composition must dispatch");
    assert_eq!(driver.live_count(), 1);

    let rows_before = obs.alloc_status_rows().await.unwrap();
    let specs_before: Vec<_> = driver.started_specs().into_iter().map(|spec| spec.alloc).collect();
    let hot_before = state.runtime.loaded_workload_lifecycle_views_for_test(&name).unwrap();
    let stored_before: std::collections::BTreeMap<TargetResource, WorkloadLifecycleView> =
        views.bulk_load("workload-lifecycle").await.unwrap();
    let mut lifecycle = state.lifecycle_events.subscribe();
    views.inject_fsync_failure();
    let failure = run_convergence_tick(&state, &name, &subject, now, 1, deadline).await;
    assert!(matches!(failure, Err(ConvergenceError::ViewPersist(_))), "seed={seed}: {failure:?}");
    assert_eq!(
        obs.alloc_status_rows().await.unwrap(),
        rows_before,
        "seed={seed}: no allocation writes"
    );
    assert_eq!(
        driver.started_specs().into_iter().map(|spec| spec.alloc).collect::<Vec<_>>(),
        specs_before
    );
    assert_eq!(driver.live_count(), 1, "seed={seed}: no subject Driver effect");
    assert!(
        matches!(lifecycle.try_recv(), Err(tokio::sync::broadcast::error::TryRecvError::Empty)),
        "seed={seed}: no lifecycle publication"
    );
    assert_eq!(state.runtime.loaded_workload_lifecycle_views_for_test(&name).unwrap(), hot_before);
    let stored_after: std::collections::BTreeMap<TargetResource, WorkloadLifecycleView> =
        views.bulk_load("workload-lifecycle").await.unwrap();
    assert_eq!(stored_after, stored_before);

    views.clear_fsync_failure();
    run_convergence_tick(&state, &name, &subject, now, 2, deadline).await.unwrap();
    assert!(running(&obs, "fsync-subject").await, "seed={seed}: same-input recovery must dispatch");
    assert_eq!(driver.live_count(), 2);
    let recovered = state.runtime.loaded_workload_lifecycle_views_for_test(&name).unwrap();
    assert_eq!(recovered.get(&subject).unwrap().observed_generation, 1);
    let durable: std::collections::BTreeMap<TargetResource, WorkloadLifecycleView> =
        views.bulk_load("workload-lifecycle").await.unwrap();
    assert_eq!(durable, recovered, "seed={seed}: recovered View is durable");
    run_convergence_tick(&state, &name, &subject, now, 3, deadline).await.unwrap();
    assert_eq!(
        driver.started_specs().len(),
        2,
        "seed={seed}: no repeated placement after recovery"
    );
}

/// Valid driving-port inputs equivalent to submit followed by a generation
/// advance before placement. The production reconciler alone authors the View
/// and resulting allocation; no stored/hot View or allocation row is seeded.
async fn runtime_job_input(state: &overdrive_control_plane::AppState, id: &str) {
    use overdrive_core::aggregate::{IntentKey, Job, WorkloadIntent};
    use overdrive_core::traits::intent_store::{IntentStore, TxnOp, TxnOutcome};
    let job = Job::from_submit(JobSpecInput {
        id: id.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/bin/sleep".to_owned(),
            args: vec!["3600".to_owned()],
        }),
    })
    .unwrap();
    let key = IntentKey::for_workload(&job.id);
    let generation = IntentKey::for_workload_generation(&job.id);
    let body = WorkloadIntent::Job(job).archive_for_store().unwrap();
    assert!(matches!(
        state
            .store
            .txn(vec![
                TxnOp::Put {
                    key: key.as_bytes().to_vec().into(),
                    value: body.as_ref().to_vec().into()
                },
                TxnOp::IncrementU64 { key: generation.as_bytes().to_vec().into() },
            ])
            .await
            .unwrap(),
        TxnOutcome::Committed
    ));
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-04. Seed 283001; submit an actual Service, hold its Driver start, and
/// drive accepted AllocStatus/ProbeResult plus public stop. Distinct reconciler
/// names for that workload share one lease until the complete runtime result
/// is consumed. A duplicate remains one pending later turn; hydration then
/// observes latest stop intent. Independent targets remain able to progress.
/// Compare complete target Views, allocation rows and final Driver membership.
#[tokio::test]
async fn same_workload_reconcilers_share_the_complete_evaluation_lease() {
    let _serial = SERVER_SCENARIO_LOCK.lock().await;
    let _ = trace_state();
    let seed = 283_001;
    let directory = tempfile::tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let clock = Arc::new(SimClock::new());
    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("local").unwrap(), seed));
    let (entered, mut entries) = mpsc::unbounded_channel();
    let driver = Arc::new(DelayedDriver {
        inner: SimDriver::with_clock(DriverType::Exec, clock.clone()),
        effect: Effect::Start,
        entered,
        release: Semaphore::new(0),
        events: Mutex::new(Vec::new()),
        exit_emission_gates: Vec::new(),
    });
    let config_dir = directory.path().join("operator");
    let config = ServerConfig {
        data_dir: data_dir.clone(),
        operator_config_dir: config_dir.clone(),
        clock: clock.clone(),
        dataplane: Some(DataplaneConfig { client_iface: "lo".into(), backend_iface: "lo".into() }),
        dataplane_override: Some(Arc::new(SimDataplane::new())),
        ..ServerConfig::new(Arc::new(SimKek::for_boot()))
    };
    let server = run_server_with_obs_and_driver(config, obs.clone(), driver.clone()).await.unwrap();
    let base = format!("https://localhost:{}", server.local_addr().await.unwrap().port());
    let client = client(&config_dir);
    let cursor = trace_cursor();

    submit_service(&client, &base, "slow-lease").await;
    for _ in 0..100 {
        advance(&clock, seed).await;
        if entries.try_recv() == Ok(Effect::Start) {
            break;
        }
    }
    assert_eq!(
        effect_targets(&driver, Effect::Start, false).len(),
        1,
        "seed={seed}: real Service start must hold the production owner"
    );
    for _ in 0..2 {
        let response =
            client.post(format!("{base}/v1/workloads/slow-lease/stop")).send().await.unwrap();
        assert!(response.status().is_success(), "seed={seed}: public stop rejected");
    }
    let status: overdrive_control_plane::api::ClusterStatus =
        client.get(format!("{base}/v1/cluster/info")).send().await.unwrap().json().await.unwrap();
    assert_eq!(status.broker.queued, 1, "seed={seed}: duplicate pending stop coalesces");

    submit(&client, &base, "lease-independent").await;
    let mut independent_while_held = false;
    for _ in 0..50 {
        advance(&clock, seed).await;
        independent_while_held |= running(&obs, "lease-independent").await;
    }
    driver.release.add_permits(1);
    let mut service_terminated = false;
    for _ in 0..200 {
        advance(&clock, seed).await;
        let rows = obs.alloc_status_rows().await.unwrap();
        service_terminated = rows.iter().any(|row| {
            row.workload_id.as_str() == "slow-lease" && row.state == AllocState::Terminated
        });
        if service_terminated && running(&obs, "lease-independent").await {
            break;
        }
    }
    let alloc = AllocationId::new("alloc-slow-lease-0").unwrap();
    let accepted_probe = ProbeResultRow {
        alloc_id: alloc.clone(),
        probe_idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        status: ProbeStatus::Pass,
        last_observed_at_unix_ms: 1,
        inferred: false,
    };
    obs.write_probe_result(accepted_probe.clone()).await.unwrap();
    for _ in 0..50 {
        advance(&clock, seed).await;
        if trace_since(cursor).iter().any(|event| {
            event.name == "convergence.evaluation.completed"
                && event_target(event) == Some("workload/slow-lease")
                && event.field("reconciler").is_some_and(|name| name.contains("service-lifecycle"))
        }) {
            break;
        }
    }
    assert_eq!(
        obs.list_probe_results_for_alloc(&alloc).await.unwrap(),
        vec![accepted_probe],
        "seed={seed}: complete accepted ProbeResult row set"
    );
    server.shutdown(Duration::from_secs(1)).await.unwrap();

    let events = trace_since(cursor);
    let mut active: Option<(String, u64)> = None;
    let mut saw_workload = false;
    let mut saw_service = false;
    for event in events.iter().filter(|event| event_target(event) == Some("workload/slow-lease")) {
        let reconciler = event.field("reconciler").unwrap_or_default().trim_matches('"');
        if event.name == "convergence.evaluation.admitted" {
            let key = (reconciler.to_owned(), event.u64_field("tick").unwrap());
            assert!(
                active.replace(key.clone()).is_none(),
                "RED scaffold (S-VLL-04 lease): seed={seed}: overlapping target owner {key:?}"
            );
            saw_workload |= reconciler == "workload-lifecycle";
            saw_service |= reconciler == "service-lifecycle";
        } else if event.name == "convergence.evaluation.completed" {
            let key = (reconciler.to_owned(), event.u64_field("tick").unwrap());
            assert_eq!(
                active.take(),
                Some(key),
                "seed={seed}: completion consumes its exact lease"
            );
        }
    }
    assert!(
        active.is_none() && saw_workload && saw_service,
        "RED scaffold (S-VLL-04 owner turns): seed={seed}: complete WorkloadLifecycle and ServiceLifecycle turns"
    );
    assert!(independent_while_held, "seed={seed}: independent target must retain progress");
    assert!(service_terminated, "seed={seed}: later hydration must observe the public stop");
    assert_eq!(driver.inner.live_count(), 1, "seed={seed}: only independent Driver member remains");

    let independent_alloc = AllocationId::new("alloc-lease-independent-0").unwrap();
    let rows = obs.alloc_status_rows().await.unwrap();
    let slow_rows: Vec<_> =
        rows.iter().filter(|row| row.workload_id.as_str() == "slow-lease").collect();
    assert_eq!(slow_rows.len(), 1, "seed={seed}: complete Service allocation row set");
    assert_eq!(slow_rows[0].alloc_id, alloc, "seed={seed}: stable allocation identity");
    assert_eq!(slow_rows[0].kind, WorkloadKind::Service, "seed={seed}: Service kind retained");
    assert_eq!(slow_rows[0].state, AllocState::Terminated, "seed={seed}: latest stop intent won");
    assert!(
        rows.iter().any(|row| {
            row.alloc_id == independent_alloc
                && row.workload_id.as_str() == "lease-independent"
                && row.state == AllocState::Running
        }),
        "seed={seed}: independent allocation row remains Running"
    );
    assert_eq!(
        effect_targets(&driver, Effect::Start, false),
        BTreeSet::from([alloc.clone(), independent_alloc]),
        "seed={seed}: complete Driver start membership"
    );
    assert_eq!(
        effect_targets(&driver, Effect::Stop, true),
        BTreeSet::from([alloc.clone()]),
        "seed={seed}: only the stopped Service left Driver membership"
    );

    let target = overdrive_core::reconcilers::TargetResource::new("workload/slow-lease").unwrap();
    let mut persisted =
        overdrive_control_plane::reconciler_runtime::ReconcilerRuntime::new_with_redb_view_store_for_test(
            &data_dir,
        )
        .unwrap();
    persisted.register(overdrive_control_plane::workload_lifecycle()).await.unwrap();
    persisted.register(overdrive_control_plane::service_lifecycle()).await.unwrap();
    let workload_name =
        overdrive_core::reconcilers::ReconcilerName::new("workload-lifecycle").unwrap();
    let service_name =
        overdrive_core::reconcilers::ReconcilerName::new("service-lifecycle").unwrap();
    let workload_view = persisted
        .loaded_workload_lifecycle_views_for_test(&workload_name)
        .unwrap()
        .get(&target)
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        workload_view,
        overdrive_reconcilers::WorkloadLifecycleView::default(),
        "seed={seed}: complete workload target View"
    );
    let service_view = persisted
        .loaded_service_lifecycle_views_for_test(&service_name)
        .unwrap()
        .get(&target)
        .cloned()
        .expect("ServiceLifecycle persisted the target View");
    let mut expected_service =
        overdrive_reconcilers::service_lifecycle::ServiceLifecycleView::default();
    expected_service.observed.insert(alloc.clone());
    assert_eq!(
        service_view, expected_service,
        "seed={seed}: later ServiceLifecycle hydration observes the stopped allocation without fabricating Stable"
    );
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-06b. Source-local convergence-owner integration complements the
/// production-server drain tests above: after every active result is consumed,
/// the one exit report equals the locked coalesced pending snapshot. A producer
/// submitting after that snapshot cannot alter the event or execute work.
/// Exercise consumed exit-event write failure through its existing four
/// attempts (50/100/200ms) before join; unread-queue draining is not promised.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn convergence_exit_report_is_the_owner_snapshot_not_final_server_backlog() {
    let _serial = SERVER_SCENARIO_LOCK.lock().await;
    let trace = trace_state();
    let seed = 283_001;
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(SimClock::new());
    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("local").unwrap(), seed));
    let first_alloc = AllocationId::new("alloc-slow-snapshot0-0").unwrap();
    let last_alloc = AllocationId::new("alloc-slow-snapshot7-0").unwrap();
    let observer_gate = Arc::new(ExitEmissionGate {
        alloc: first_alloc.clone(),
        entered: Notify::new(),
        entered_flag: AtomicBool::new(false),
        release: Semaphore::new(0),
    });
    let snapshot_gate = Arc::new(ExitEmissionGate {
        alloc: last_alloc,
        entered: Notify::new(),
        entered_flag: AtomicBool::new(false),
        release: Semaphore::new(0),
    });
    let (entered, mut entries) = mpsc::unbounded_channel();
    let driver = Arc::new(DelayedDriver {
        inner: SimDriver::with_clock(DriverType::Exec, clock.clone()),
        effect: Effect::Start,
        entered,
        release: Semaphore::new(0),
        events: Mutex::new(Vec::new()),
        exit_emission_gates: vec![observer_gate.clone(), snapshot_gate.clone()],
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
    let base = format!("https://localhost:{}", server.local_addr().await.unwrap().port());
    let client = client(&config_dir);
    let cursor = trace_cursor();
    for index in 0..8 {
        submit(&client, &base, &format!("slow-snapshot{index}")).await;
    }
    let mut active = 0;
    for _ in 0..100 {
        advance(&clock, seed).await;
        while entries.try_recv() == Ok(Effect::Start) {
            active += 1;
        }
        if active == 8 {
            break;
        }
    }
    for _ in 0..2 {
        let response =
            client.post(format!("{base}/v1/workloads/slow-snapshot0/stop")).send().await.unwrap();
        assert!(response.status().is_success(), "seed={seed}: repeated stop rejected");
    }
    submit(&client, &base, "snapshot-pending").await;

    if active != 8 {
        observer_gate.release.add_permits(1);
        snapshot_gate.release.add_permits(1);
        driver.release.add_permits(8);
        server.shutdown(Duration::from_secs(1)).await.unwrap();
        panic!(
            "RED scaffold (S-VLL-06b admission): seed={seed}: owner held {active}/8 evaluations before snapshot testing"
        );
    }

    driver.inner.inject_exit_after(&first_alloc, Duration::ZERO, ExitKind::CleanExit);
    let mut shutdown = tokio::spawn(server.shutdown(Duration::from_secs(1)));
    driver.release.add_permits(1);
    observer_gate.wait_entered().await;
    for attempt in 0..4 {
        obs.inject_write_failure(ObservationStoreError::Unreachable {
            peer: format!("consumed-exit-attempt-{attempt}"),
        });
    }
    observer_gate.release.add_permits(1);
    for _ in 0..20 {
        advance(&clock, seed).await;
        let events = trace_since(cursor);
        let exhausted = events.iter().any(|event| {
            event.metadata_target == "overdrive::exit_observer"
                && event.u64_field("attempts") == Some(4)
        });
        if exhausted {
            break;
        }
    }
    let retry_events = trace_since(cursor);
    let backoffs: Vec<_> = retry_events
        .iter()
        .filter(|event| event.metadata_target == "overdrive::exit_observer")
        .filter_map(|event| event.u64_field("backoff_ms"))
        .collect();
    assert_eq!(
        backoffs,
        vec![50, 100, 200],
        "seed={seed}: consumed observer event finishes its existing retry schedule"
    );
    assert!(
        retry_events.iter().any(|event| {
            event.metadata_target == "overdrive::exit_observer"
                && event.u64_field("attempts") == Some(4)
        }),
        "seed={seed}: consumed observer event reaches the fourth attempt before join"
    );

    for released in 2..=7 {
        driver.release.add_permits(1);
        for _ in 0..100 {
            advance(&clock, seed).await;
            if evaluation_keys(&trace_since(cursor), "convergence.evaluation.completed").len()
                >= released
            {
                break;
            }
        }
        assert!(
            !shutdown.is_finished(),
            "seed={seed}: shutdown discarded active result {released}"
        );
    }
    driver.release.add_permits(1);
    snapshot_gate.wait_entered().await;
    for _ in 0..10 {
        advance(&clock, seed).await;
    }
    let before_snapshot: overdrive_control_plane::api::ClusterStatus =
        client.get(format!("{base}/v1/cluster/info")).send().await.unwrap().json().await.unwrap();
    assert!(
        before_snapshot.broker.queued >= 3 && before_snapshot.broker.cancelled >= 1,
        "seed={seed}: controlled coalesced pending set must exist before the owner snapshot"
    );

    let drain_gate = Arc::new(DrainEventGate::default());
    *trace.drain_gate.lock().expect("drain gate mutex") = Some(drain_gate.clone());
    snapshot_gate.release.add_permits(1);
    if tokio::time::timeout(Duration::from_secs(5), drain_gate.wait_reached()).await.is_err() {
        let _ = tokio::time::timeout(Duration::from_secs(10), &mut shutdown).await;
        panic!(
            "RED scaffold (S-VLL-06b trace): seed={seed}: convergence owner emitted no drain snapshot event"
        );
    }
    submit(&client, &base, "snapshot-late").await;
    drain_gate.release();
    tokio::time::timeout(Duration::from_secs(10), shutdown).await.unwrap().unwrap().unwrap();

    let events = trace_since(cursor);
    let drains = drain_events(&events);
    assert_eq!(drains.len(), 1, "RED scaffold (S-VLL-06b report): one owner exit report");
    assert!(drains[0].u64_field("elapsed_ms").is_some(), "seed={seed}: drain elapsed field");
    assert_eq!(
        drains[0].u64_field("pending_at_exit"),
        Some(before_snapshot.broker.queued),
        "seed={seed}: report is the locked owner snapshot"
    );
    assert_eq!(drains[0].u64_field("admitted_at_close"), Some(8), "seed={seed}");
    assert_eq!(drains[0].u64_field("completed_during_drain"), Some(8), "seed={seed}");
    assert!(
        events.iter().all(|event| {
            event_target(event) != Some("workload/snapshot-late")
                || (event.name != "convergence.evaluation.admitted"
                    && event.name != "convergence.evaluation.completed")
        }),
        "seed={seed}: post-snapshot submission must remain unexecuted"
    );
    assert!(
        driver.events.lock().iter().all(|event| !matches!(
            event,
            DriverEvent::Entered(_, alloc) if alloc.as_str().starts_with("alloc-snapshot-late")
        )),
        "seed={seed}: post-snapshot work reached Driver"
    );
    assert!(
        obs.alloc_status_rows()
            .await
            .unwrap()
            .iter()
            .all(|row| row.workload_id.as_str() != "snapshot-late"),
        "seed={seed}: post-snapshot work acquired an allocation row"
    );
}
