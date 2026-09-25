//! Reusable direct-handler/public-API conformance harness.
//!
//! The server is composed through its exported production handler, while every
//! workload/admission request enters through the public HTTPS API. `nix` and
//! `overdrive-netlink` provide typed host observations; product application
//! internals are never called to submit or mutate workload state.
//!
//! # Composition (D-295-R16)
//!
//! [`DirectHandlerHarness::start`] composes
//! `run_server_with_obs_and_driver(ServerConfig::new(kek, mtls_intercept,
//! guest_dns), obs, driver, vm_host_state, shared_guest_network,
//! guest_network_exec, vm_cgroups)` with the required protection and DNS ports
//! supplied as `SimMtlsIntercept` and `SimGuestDnsFactory`, the caller's
//! `SimSharedGuestNetworkOwner`, and `vm_cgroups` as a `CgroupManager` over a
//! `SimCgroupFs`, so no in-process case writes a real `cgroup.kill`.
//!
//! # In-process observation (`test-scenarios.md` § *In-process observation*)
//!
//! The harness keeps `wiring.gate()` and `wiring.supervisor()` before moving the
//! wiring into the handler. A scenario advances the injected clock in
//! [`OBSERVATION_STEP`]s; after each step the harness polls
//! `handle.shutdown_requested()` and `gate.claim_release()` exactly once and
//! drops each future, and reads `recovery_progress()` and `is_boot_closed()`
//! directly. No observation future is held across a step or awaited.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unused_async,
    reason = "the conformance harness uses fail-fast diagnostic assertions like an integration test"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::net::SocketAddr;
use std::os::unix::fs::FileTypeExt as _;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::process::Output;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context as TaskContext, Poll, Waker};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use overdrive_control_plane::guest_network::GuestNetworkOperation;
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server_with_obs_and_driver};
use overdrive_core::guest_network::{
    GuestNetworkExecGate, GuestNetworkExecSupervisor, GuestNetworkExecWiring, ServeShutdownRequest,
    SharedGuestNetworkFailStop, SharedGuestNetworkRecovery,
};
use overdrive_core::id::NodeId;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;

/// ADR-0124's accepted shared-network recovery cadence, stated as that
/// contract and measured on the injected clock.
///
/// The control plane's `SHARED_NETWORK_*` constants are private items of its
/// crate root; the in-process lane never names them and none is widened for it
/// (FD 4109-4145). The E18-derived call bounds are not part of this contract.
pub mod accepted_cadence {
    use std::time::Duration;

    /// One full audit of the shared-network owners starts every period.
    pub const AUDIT_PERIOD: Duration = Duration::from_secs(1);
    /// One recovery attempt runs every period while recovering.
    pub const ATTEMPT_PERIOD: Duration = Duration::from_millis(250);
    /// The bounded recovery window after detection.
    pub const RECOVERY_DEADLINE: Duration = Duration::from_secs(5);
    /// The attempt budget inside the recovery window.
    pub const RECOVERY_ATTEMPTS: u32 = 20;
}

/// Injected-time step between in-process observations. It divides the accepted
/// attempt and audit periods, so every supervisor wake lands on a step.
pub const OBSERVATION_STEP: Duration = Duration::from_millis(50);

/// Host-scheduling settle after each injected-clock step, so the woken
/// production tasks run before the observation. It is integration-host
/// scheduling, never a simulated measurement: every oracle is stated in
/// injected time.
const STEP_SETTLE: Duration = Duration::from_millis(10);

async fn settle_after_step() {
    tokio::time::sleep(STEP_SETTLE).await;
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

/// Poll `future` exactly once with a no-op waker and drop it.
///
/// This is the `FutureExt::now_or_never` discipline, written over `std` so the
/// harness adds no dependency.
fn now_or_never<F: Future>(future: F) -> Option<F::Output> {
    let mut future = pin!(future);
    match future.as_mut().poll(&mut TaskContext::from_waker(Waker::noop())) {
        Poll::Ready(output) => Some(output),
        Poll::Pending => None,
    }
}

/// Guest-command admission as one non-blocking `claim_release()` poll reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// The poll is pending: the gate is `BootClosed` or `Recovering`.
    Closed,
    /// The poll granted a claim, which the harness dropped at once.
    Open,
    /// The poll returned `None`: the gate is `FailStop`.
    FailStop,
}

/// One observation of a live handler.
#[derive(Debug, Clone)]
pub struct StepObservation {
    /// Injected time since the trajectory began; zero outside a trajectory.
    pub at: Duration,
    /// The typed request `ServerHandle::shutdown_requested` returned on this
    /// poll. A request is returned once; later polls do not repeat it.
    pub request: Option<ServeShutdownRequest>,
    /// Guest-command admission.
    pub admission: Admission,
    /// `recovery_progress()`: `Some` only while recovering.
    pub recovery: Option<SharedGuestNetworkRecovery>,
    /// `is_boot_closed()`.
    pub boot_closed: bool,
}

impl StepObservation {
    /// The fail-stop receipt this observation received, if any.
    #[must_use]
    pub fn fail_stop(&self) -> Option<&SharedGuestNetworkFailStop> {
        self.request.as_ref().map(|ServeShutdownRequest::SharedGuestNetwork(fail_stop)| fail_stop)
    }
}

/// The observations one stepped run of the injected clock produced, in order.
#[derive(Debug, Clone, Default)]
pub struct Trajectory {
    pub observations: Vec<StepObservation>,
}

impl Trajectory {
    /// The first observation satisfying `predicate`.
    #[must_use]
    pub fn first<P>(&self, predicate: P) -> Option<&StepObservation>
    where
        P: FnMut(&&StepObservation) -> bool,
    {
        self.observations.iter().find(predicate)
    }

    /// Every fail-stop receipt received during the trajectory.
    #[must_use]
    pub fn fail_stops(&self) -> Vec<&SharedGuestNetworkFailStop> {
        self.observations.iter().filter_map(StepObservation::fail_stop).collect()
    }

    /// Whether admission opened again after it had left `Open`.
    #[must_use]
    pub fn reopened_after_closing(&self) -> bool {
        self.observations
            .iter()
            .skip_while(|observation| observation.admission == Admission::Open)
            .any(|observation| observation.admission == Admission::Open)
    }
}

/// What a freshly constructed handler had completed when its HTTPS API first
/// became reachable, captured before any request is sent to it.
#[derive(Debug, Clone)]
pub struct BootEvidence {
    /// Admission, recovery, and request state at that moment.
    pub observation: StepObservation,
    /// The handler's shared-owner port calls so far, in call order.
    pub owner_calls: Vec<GuestNetworkOperation>,
    /// `guest_network.shared_owner_boot_phase` `(phase, transition)` pairs the
    /// handler emitted, in emission order.
    pub boot_phases: Vec<(String, String)>,
}

/// Append-only, timestamped diagnostic history shared by a harness and every
/// handler it starts.
#[derive(Clone)]
struct Diagnostics {
    started: Instant,
    entries: Arc<Mutex<Vec<String>>>,
}

impl Diagnostics {
    fn new() -> Self {
        Self { started: Instant::now(), entries: Arc::new(Mutex::new(Vec::new())) }
    }

    fn record(&self, phase: &str, observation: &str) {
        self.entries.lock().expect("diagnostic history lock").push(format!(
            "elapsed_ms={} phase={phase} observation={observation}",
            self.started.elapsed().as_millis()
        ));
    }

    fn history(&self) -> Vec<String> {
        self.entries.lock().expect("diagnostic history lock").clone()
    }
}

/// One structured tracing event retained append-only for the conformance run.
#[derive(Debug, Clone, Default)]
pub struct TraceEvent {
    pub name: String,
    pub fields: BTreeMap<String, String>,
}

#[derive(Default)]
struct TraceVisitor {
    fields: BTreeMap<String, String>,
}

impl Visit for TraceVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}").trim_matches('"').to_owned());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields.insert(field.name().to_owned(), value.to_owned());
    }
}

/// Append-only structured event collector shared by direct-handler instances.
#[derive(Clone, Default)]
pub struct TraceHistory {
    inner: Arc<Mutex<Vec<TraceEvent>>>,
}

impl TraceHistory {
    /// Install this collector for the conformance test process.
    #[must_use]
    pub fn install_global() -> Self {
        let history = Self::default();
        tracing::subscriber::set_global_default(
            tracing_subscriber::registry().with(history.clone()),
        )
        .expect("install conformance tracing collector once");
        history
    }

    /// Snapshot all events without consuming or overwriting earlier evidence.
    #[must_use]
    pub fn snapshot(&self) -> Vec<TraceEvent> {
        self.inner.lock().expect("trace history lock").clone()
    }

    /// A cursor at the current end of the history, for [`Self::since`].
    #[must_use]
    pub fn mark(&self) -> usize {
        self.inner.lock().expect("trace history lock").len()
    }

    /// Every event recorded after `mark`, in record order.
    #[must_use]
    pub fn since(&self, mark: usize) -> Vec<TraceEvent> {
        self.inner.lock().expect("trace history lock").iter().skip(mark).cloned().collect()
    }
}

impl<S> Layer<S> for TraceHistory
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut visitor = TraceVisitor::default();
        event.record(&mut visitor);
        self.inner
            .lock()
            .expect("trace history lock")
            .push(TraceEvent { name: event.metadata().name().to_owned(), fields: visitor.fields });
    }
}

/// Run an external substrate command only where no typed Rust surface exists.
#[must_use]
pub fn command_output(program: &str, args: &[&str]) -> Output {
    std::process::Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("run {program} {args:?}: {error}"))
}

/// Assert the qualified native-metal substrate without using the product CLI.
pub fn assert_native_metal() {
    assert_eq!(std::env::consts::OS, "linux", "native Linux required");
    assert_eq!(std::env::consts::ARCH, "x86_64", "native x86_64 required");
    assert!(nix::unistd::geteuid().is_root(), "native-metal conformance requires root");
    let kvm = std::fs::metadata("/dev/kvm").expect("stat /dev/kvm");
    assert!(kvm.file_type().is_char_device(), "/dev/kvm must be a character device");
    let virtualization = command_output("systemd-detect-virt", &[]);
    assert!(
        virtualization.status.success()
            && String::from_utf8_lossy(&virtualization.stdout).trim() == "none",
        "virtualized substrate refused: {}",
        String::from_utf8_lossy(&virtualization.stdout)
    );
}

/// Poll one public or kernel observation until it is true or its deadline expires.
pub fn wait_until<F>(bound: Duration, description: &str, mut predicate: F)
where
    F: FnMut() -> bool,
{
    let deadline = Instant::now() + bound;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        thread::sleep(Duration::from_millis(200));
    }
    panic!("{description} exceeded {bound:?}");
}

/// Directory-entry names, with absent directories represented honestly as empty.
#[must_use]
pub fn directory_entries(path: &Path) -> BTreeSet<String> {
    match std::fs::read_dir(path) {
        Ok(entries) => entries
            .map(|entry| {
                entry
                    .unwrap_or_else(|error| panic!("read {} entry: {error}", path.display()))
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
        Err(error) => panic!("read {}: {error}", path.display()),
    }
}

/// Managed TAP/veth link names observed without shelling out to `ip`.
#[must_use]
pub fn managed_links() -> BTreeSet<String> {
    directory_entries(Path::new("/sys/class/net"))
        .into_iter()
        .filter(|name| name.starts_with("ovd-tp-") || name.starts_with("ovd-hv-"))
        .collect()
}

/// Network namespaces observed from the kernel's named-netns mount directory.
#[must_use]
pub fn network_namespaces() -> BTreeSet<String> {
    directory_entries(Path::new("/var/run/netns"))
}

/// Typed persistent-TAP observation through the host netlink adapter.
pub fn persistent_tap_state(name: &str) -> overdrive_netlink::TapLinkState {
    let name = name.to_owned();
    overdrive_netlink::block_on_host_netlink(move || async move {
        let client = overdrive_netlink::Client::new()?;
        client.observe_persistent_tap(&name).await
    })
    .expect("observe persistent TAP through overdrive-netlink")
}

/// Independent typed ifindex lookup through `nix`.
#[must_use]
pub fn interface_index(name: &str) -> Option<u32> {
    match nix::net::if_::if_nametoindex(name) {
        Ok(index) => Some(index),
        Err(nix::errno::Errno::ENODEV | nix::errno::Errno::ENXIO) => None,
        Err(error) => panic!("observe ifindex for {name}: {error}"),
    }
}

/// Reusable public/kernel cleanup snapshot for one handler instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupObservation {
    managed_links: BTreeSet<String>,
    link_indices: BTreeMap<String, u32>,
    tap_states: BTreeMap<String, overdrive_netlink::TapLinkState>,
    network_namespaces: BTreeSet<String>,
    directories: Vec<(PathBuf, BTreeSet<String>)>,
}

impl CleanupObservation {
    #[must_use]
    pub fn capture(directories: &[&Path]) -> Self {
        let managed_links = managed_links();
        Self {
            link_indices: managed_links
                .iter()
                .filter_map(|name| interface_index(name).map(|index| (name.clone(), index)))
                .collect(),
            tap_states: managed_links
                .iter()
                .filter(|name| name.starts_with("ovd-tp-"))
                .map(|name| (name.clone(), persistent_tap_state(name)))
                .collect(),
            managed_links,
            network_namespaces: network_namespaces(),
            directories: directories
                .iter()
                .map(|path| ((*path).to_path_buf(), directory_entries(path)))
                .collect(),
        }
    }

    #[must_use]
    pub fn is_restored(&self) -> bool {
        let managed_links = managed_links();
        managed_links == self.managed_links
            && managed_links
                .iter()
                .filter_map(|name| interface_index(name).map(|index| (name.clone(), index)))
                .collect::<BTreeMap<_, _>>()
                == self.link_indices
            && managed_links
                .iter()
                .filter(|name| name.starts_with("ovd-tp-"))
                .map(|name| (name.clone(), persistent_tap_state(name)))
                .collect::<BTreeMap<_, _>>()
                == self.tap_states
            && network_namespaces() == self.network_namespaces
            && self.directories.iter().all(|(path, expected)| directory_entries(path) == *expected)
    }
}

/// HTTPS client built from the server-written trust triple.
#[derive(Debug, Clone)]
pub struct PublicApi {
    client: reqwest::Client,
    endpoint: String,
}

impl PublicApi {
    pub fn from_trust_config(path: &Path) -> Result<Self, String> {
        let source = std::fs::read_to_string(path)
            .map_err(|error| format!("read trust config {}: {error}", path.display()))?;
        let parsed: toml::Value =
            toml::from_str(&source).map_err(|error| format!("parse trust config: {error}"))?;
        let current = parsed
            .get("current-context")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| "trust config has no current-context".to_owned())?;
        let context = parsed
            .get("contexts")
            .and_then(toml::Value::as_array)
            .and_then(|contexts| {
                contexts.iter().find(|context| {
                    context.get("name").and_then(toml::Value::as_str) == Some(current)
                })
            })
            .ok_or_else(|| format!("trust config has no context {current:?}"))?;
        let field = |name: &str| {
            context
                .get(name)
                .and_then(toml::Value::as_str)
                .ok_or_else(|| format!("trust context has no {name:?}"))
        };
        let endpoint = field("endpoint")?.trim_end_matches('/').to_owned();
        let ca = BASE64.decode(field("ca")?).map_err(|error| format!("decode CA: {error}"))?;
        let cert =
            BASE64.decode(field("crt")?).map_err(|error| format!("decode client cert: {error}"))?;
        let key =
            BASE64.decode(field("key")?).map_err(|error| format!("decode client key: {error}"))?;
        let root = reqwest::Certificate::from_pem(&ca)
            .map_err(|error| format!("parse CA certificate: {error}"))?;
        let mut identity_pem = cert;
        if !identity_pem.ends_with(b"\n") {
            identity_pem.push(b'\n');
        }
        identity_pem.extend_from_slice(&key);
        let identity = reqwest::Identity::from_pem(&identity_pem)
            .map_err(|error| format!("parse client identity: {error}"))?;
        let client = reqwest::Client::builder()
            .add_root_certificate(root)
            .identity(identity)
            .https_only(true)
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|error| format!("build public API client: {error}"))?;
        Ok(Self { client, endpoint })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.endpoint, path.trim_start_matches('/'))
    }

    pub async fn get(&self, path: &str) -> reqwest::Result<reqwest::Response> {
        self.client.get(self.url(path)).send().await
    }

    pub async fn post_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> reqwest::Result<reqwest::Response> {
        self.client
            .post(self.url(path))
            .header(reqwest::header::ACCEPT, "application/json")
            .json(body)
            .send()
            .await
    }

    /// Submit one workload through the public API and assert its identity.
    pub async fn submit_workload(&self, id: &str, body: &serde_json::Value) {
        let response = self
            .post_json("/v1/workloads", body)
            .await
            .expect("public submit request reaches the handler");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let response: serde_json::Value = response.json().await.expect("decode submit response");
        assert_eq!(response["workload_id"], id);
        assert_eq!(response["outcome"], "inserted");
    }

    /// Observe one desired/one Running replica through the public API.
    pub async fn one_replica_running(&self, id: &str) -> bool {
        let Ok(response) = self.get(&format!("/v1/allocs?job={id}")).await else {
            return false;
        };
        if !response.status().is_success() {
            return false;
        }
        let Ok(status) = response.json::<serde_json::Value>().await else {
            return false;
        };
        status["replicas_desired"] == 1 && status["replicas_running"] == 1
    }

    /// Stop one workload through the public API.
    pub async fn stop_workload(&self, id: &str) {
        let response = self
            .post_json(&format!("/v1/workloads/{id}/stop"), &serde_json::json!({}))
            .await
            .expect("public stop request reaches the handler");
        assert!(response.status().is_success());
    }
}

/// Reusable composition owner for successive server-handler instances over
/// the same data/config roots.
pub struct DirectHandlerHarness {
    _root: tempfile::TempDir,
    data_dir: PathBuf,
    config_dir: PathBuf,
    sequence: AtomicU64,
    diagnostics: Diagnostics,
}

impl DirectHandlerHarness {
    #[must_use]
    pub fn new() -> Self {
        let root = tempfile::tempdir().expect("conformance tempdir");
        let data_dir = root.path().join("data");
        let config_dir = root.path().join("config");
        std::fs::create_dir_all(&data_dir).expect("create conformance data dir");
        std::fs::create_dir_all(&config_dir).expect("create conformance config dir");
        Self {
            _root: root,
            data_dir,
            config_dir,
            sequence: AtomicU64::new(0),
            diagnostics: Diagnostics::new(),
        }
    }

    /// Append one non-destructive diagnostic observation for this run.
    pub fn record(&self, phase: &str, observation: &str) {
        self.diagnostics.record(phase, observation);
    }

    /// Snapshot the append-only diagnostic history without consuming it.
    #[must_use]
    pub fn diagnostic_history(&self) -> Vec<String> {
        self.diagnostics.history()
    }

    /// Construct a fresh handler over this harness's retained data and config
    /// roots and capture, before any request reaches it, what it completed
    /// while booting: its admission state, its shared-owner port calls, and the
    /// boot phases it emitted into `trace`.
    pub async fn start_fresh(
        &self,
        owner: Arc<SimSharedGuestNetworkOwner>,
        trace: &TraceHistory,
    ) -> (DirectHandlerInstance, BootEvidence) {
        let mark = trace.mark();
        let mut instance = self.start(owner).await;
        let observation = instance.observe();
        let owner_calls = instance.owner().calls();
        let boot_phases = trace
            .since(mark)
            .into_iter()
            .filter(|event| event.name == "guest_network.shared_owner_boot_phase")
            .map(|event| {
                let field = |name: &str| event.fields.get(name).cloned().unwrap_or_default();
                (field("phase"), field("transition"))
            })
            .collect();
        let evidence = BootEvidence { observation, owner_calls, boot_phases };
        self.record("fresh_handler_boot_evidence", &format!("{evidence:?}"));
        (instance, evidence)
    }

    pub async fn start(&self, owner: Arc<SimSharedGuestNetworkOwner>) -> DirectHandlerInstance {
        let clock = Arc::new(SimClock::new());
        let clock_port: Arc<dyn overdrive_core::traits::clock::Clock> = clock.clone();
        let config = ServerConfig {
            bind: "127.0.0.1:0".parse().expect("loopback bind"),
            data_dir: self.data_dir.clone(),
            operator_config_dir: self.config_dir.clone(),
            clock: Arc::clone(&clock_port),
            dataplane: Some(overdrive_control_plane::dataplane_config::DataplaneConfig {
                client_iface: "lo".to_owned(),
                backend_iface: "lo".to_owned(),
            }),
            dataplane_override: Some(Arc::new(
                overdrive_sim::adapters::dataplane::SimDataplane::new(),
            )),
            ..ServerConfig::new(
                Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
                std::sync::Arc::new(overdrive_sim::adapters::SimMtlsIntercept::new()),
                std::sync::Arc::new(overdrive_sim::adapters::SimGuestDnsFactory::default()),
            )
        };
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);
        self.record("handler_start_attempt", &format!("sequence={sequence}"));
        let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
            NodeId::new(&format!("shared-guest-network-handler-{sequence}")).expect("node id"),
            sequence,
        ));
        let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Vm));
        // The wiring moves into the handler; its paired gate and supervisor
        // capabilities are kept first for the in-process observations.
        let wiring = GuestNetworkExecWiring::new(clock_port);
        let gate = wiring.gate();
        let supervisor = wiring.supervisor();
        let owner_port: Arc<dyn overdrive_control_plane::guest_network::SharedGuestNetworkOwner> =
            owner.clone();
        let vm_host_state = Arc::new(overdrive_sim::adapters::vm_host_state::SimVmHostState::new());
        let handle = run_server_with_obs_and_driver(
            config,
            obs,
            driver,
            vm_host_state,
            owner_port,
            wiring,
            overdrive_worker::cgroup_manager::CgroupManager::new(
                std::path::PathBuf::from("/sys/fs/cgroup"),
                std::sync::Arc::new(overdrive_sim::adapters::SimCgroupFs::new()),
            ),
        )
        .await
        .expect("start production server handler");
        let address = handle.local_addr().await.expect("server handler bound address");
        let trust_path = self.config_dir.join(".overdrive/config");
        let api = PublicApi::from_trust_config(&trust_path).expect("load server trust triple");
        self.record("handler_start_complete", &format!("sequence={sequence} address={address}"));
        DirectHandlerInstance {
            handle: Some(handle),
            api,
            owner,
            clock,
            address,
            gate,
            supervisor,
            sequence,
            diagnostics: self.diagnostics.clone(),
        }
    }
}

impl Default for DirectHandlerHarness {
    fn default() -> Self {
        Self::new()
    }
}

/// One live production server-handler instance.
pub struct DirectHandlerInstance {
    handle: Option<ServerHandle>,
    api: PublicApi,
    owner: Arc<SimSharedGuestNetworkOwner>,
    clock: Arc<SimClock>,
    address: SocketAddr,
    gate: Arc<GuestNetworkExecGate>,
    supervisor: Arc<GuestNetworkExecSupervisor>,
    sequence: u64,
    diagnostics: Diagnostics,
}

impl DirectHandlerInstance {
    #[must_use]
    pub const fn api(&self) -> &PublicApi {
        &self.api
    }

    #[must_use]
    pub const fn owner(&self) -> &Arc<SimSharedGuestNetworkOwner> {
        &self.owner
    }

    #[must_use]
    pub const fn clock(&self) -> &Arc<SimClock> {
        &self.clock
    }

    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    /// Await the typed request. Unlike [`Self::observe`] this holds the
    /// observation future across clock steps.
    pub async fn shutdown_requested(&mut self) -> ServeShutdownRequest {
        self.handle.as_mut().expect("live server handle").shutdown_requested().await
    }

    /// Observe the handler once without advancing the clock.
    ///
    /// The request is polled before the gate: the supervisor writes `FailStop`
    /// before it sends, so a received request is never paired with an earlier
    /// gate reading.
    pub fn observe(&mut self) -> StepObservation {
        let request =
            now_or_never(self.handle.as_mut().expect("live server handle").shutdown_requested());
        let admission = match now_or_never(self.gate.claim_release()) {
            Some(Some(claim)) => {
                drop(claim);
                Admission::Open
            }
            Some(None) => Admission::FailStop,
            None => Admission::Closed,
        };
        StepObservation {
            at: Duration::ZERO,
            request,
            admission,
            recovery: self.supervisor.recovery_progress(),
            boot_closed: self.supervisor.is_boot_closed(),
        }
    }

    /// Advance the injected clock in [`OBSERVATION_STEP`]s for at most
    /// `horizon`, observing once after each step, and stop after the first
    /// observation `done` accepts. Every observation is appended to the
    /// diagnostic history.
    pub async fn step_until<F>(&mut self, horizon: Duration, mut done: F) -> Trajectory
    where
        F: FnMut(&StepObservation) -> bool + Send,
    {
        let mut trajectory = Trajectory::default();
        let mut at = Duration::ZERO;
        while at < horizon {
            self.clock.tick(OBSERVATION_STEP);
            at += OBSERVATION_STEP;
            settle_after_step().await;
            let observation = StepObservation { at, ..self.observe() };
            self.diagnostics
                .record("step_observation", &format!("sequence={} {observation:?}", self.sequence));
            let finished = done(&observation);
            trajectory.observations.push(observation);
            if finished {
                break;
            }
        }
        trajectory
    }

    /// Drive the injected production clock while observing admission only
    /// through the public allocation-status API.
    pub async fn wait_running(&self, id: &str, bound: Duration) {
        tokio::time::timeout(bound, async {
            loop {
                self.clock.tick(Duration::from_millis(250));
                tokio::task::yield_now().await;
                if self.api.one_replica_running(id).await {
                    return;
                }
            }
        })
        .await
        .expect("public API admission reaches one desired/one running replica");
    }

    pub async fn shutdown(mut self, deadline: Duration) {
        self.handle
            .take()
            .expect("live server handle")
            .shutdown(deadline)
            .await
            .expect("server handler shutdown converges");
    }

    /// Attempt handler shutdown with an explicit outer deadline, retaining a
    /// stable black-box success/failure disposition for timeout scenarios.
    pub async fn shutdown_result(mut self, deadline: Duration) -> Result<(), String> {
        self.handle
            .take()
            .expect("live server handle")
            .shutdown(deadline)
            .await
            .map_err(|error| error.to_string())
    }
}
