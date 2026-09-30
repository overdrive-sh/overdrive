//! S-ND295-59 — `GET /v1/allocs` reports a stuck stop as cleanup-pending and
//! never counts it as a running replica (GH #295, D-295-R20 server projection,
//! FD § "[REF] Operator status — network cleanup pending (D-295-R20) — ACCEPTED 2026-09-24 (operator behaviour user ruling of the same date)" (the server projection)).
//!
//! In-process lane (Lima root, `integration-tests`): the production composition
//! `run_server_with_obs_and_driver(ServerConfig::new(kek, mtls_intercept,
//! guest_dns), obs, driver, vm_host_state, shared_guest_network,
//! guest_network_exec, vm_cgroups)` (FD § "EXEC-close linearization" (the `run_server_with_obs_and_driver(s)` signatures)) with a `SimDriver`, the
//! `SimSharedGuestNetworkOwner`, `SimGuestDnsFactory`, and `vm_cgroups` over
//! `SimCgroupFs`. Workloads are deployed and stopped, and allocations read,
//! through the public HTTPS API only.
//!
//! Faults enter through driven ports: the required `ServerConfig.mtls_intercept`
//! is a test-local intercept over `SimMtlsIntercept` whose
//! `remove_allocation_elements` fails while armed, and
//! `SimDriver::inject_exit_after(.., Crashed)` ends a VM. Every row, lease
//! transition, and projection the oracle reads is authored by production.
//!
//! The lease state is observed only through the pinned lease events
//! `guest_network.lease_retired { alloc }` and `guest_network.lease_released
//! { alloc }` (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the lease events)); the pool is crate-private. The
//! in-process server runs on every worker thread of the multi-thread runtime,
//! so the capture is the process's global subscriber; nextest runs one test
//! per process, so exactly one test installs it.
//!
//! The convergence loop runs on an injected `SimClock` that the test advances.

#![allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use overdrive_control_plane::api::{
    AllocStateWire, AllocStatusResponse, AllocStatusRowBody, SubmitWorkloadRequest,
};
use overdrive_control_plane::dns_responder::GuestDnsFactory;
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server_with_obs_and_driver};
use overdrive_core::aggregate::{DriverInput, JobSpecInput, ResourcesInput, VmInput};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::guest_network::GuestNetworkExecWiring;
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType, ExitKind};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::vm_host_state::SimVmHostState;
use overdrive_sim::adapters::{SimCgroupFs, SimGuestDnsFactory, SimKek, SimMtlsIntercept};
use overdrive_worker::cgroup_manager::CgroupManager;
use overdrive_worker::mtls_intercept::{
    InterceptElementKey, InterceptElementOperation, InterceptError, InterceptPostcondition,
    InterceptSet, NetlinkError, Result as InterceptResult,
};
use overdrive_worker::mtls_intercept_port::{
    InterceptGuard, InterceptMembers, InterceptState, MtlsIntercept,
};
use parking_lot::Mutex;
use tempfile::TempDir;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::workload_lifecycle::wait::advance_and_settle;

/// The convergence tick cadence the injected clock is advanced by.
const TICK: Duration = Duration::from_millis(100);
/// Upper bound on injected ticks any one wait may take (30 s of logical time).
const MAX_TICKS: usize = 300;

// ---------------------------------------------------------------------------
// Test-local intercept: the required `ServerConfig.mtls_intercept`.
// ---------------------------------------------------------------------------

/// The `InterceptError` the armed removal fault returns for `source_addr`.
fn injected_removal_error(source_addr: Ipv4Addr) -> InterceptError {
    InterceptError::NftElementUpdateFailed {
        set: InterceptSet::ManagedGuestIps,
        operation: InterceptElementOperation::Delete,
        key: InterceptElementKey::Address(source_addr),
        source: NetlinkError::nft(
            "shared-element-remove",
            std::io::Error::from_raw_os_error(libc::EBUSY),
        ),
    }
}

/// The listener type `MtlsIntercept::bind_transparent` returns. The DELIVER
/// step that carries B-7 (05-01 at the latest) changes it to
/// `Arc<dyn InterceptListener>` (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the pinned `bind_transparent` signature)); the delegation below is
/// unchanged by that step.
type BoundListener = std::net::TcpListener;

/// Test-local `MtlsIntercept` delegating to an inner `SimMtlsIntercept`;
/// `remove_allocation_elements` fails with [`injected_removal_error`] while
/// armed and counts every call.
struct RemovalFaultIntercept {
    sim: SimMtlsIntercept,
    armed: AtomicBool,
    removals: Mutex<Vec<Ipv4Addr>>,
}

impl RemovalFaultIntercept {
    fn new() -> Self {
        Self {
            sim: SimMtlsIntercept::new(),
            armed: AtomicBool::new(false),
            removals: Mutex::new(Vec::new()),
        }
    }

    fn arm(&self, armed: bool) {
        self.armed.store(armed, Ordering::SeqCst);
    }

    fn removal_calls(&self) -> usize {
        self.removals.lock().len()
    }
}

impl MtlsIntercept for RemovalFaultIntercept {
    fn bind_transparent(&self, addr: SocketAddrV4) -> InterceptResult<BoundListener> {
        self.sim.bind_transparent(addr)
    }

    fn converge_shared(
        &self,
        prior: Option<&InterceptPostcondition>,
        leg_f: SocketAddrV4,
        leg_c: SocketAddrV4,
    ) -> InterceptResult<Box<dyn InterceptGuard>> {
        self.sim.converge_shared(prior, leg_f, leg_c)
    }

    fn observe_shared(&self) -> InterceptResult<Option<InterceptPostcondition>> {
        self.sim.observe_shared()
    }

    fn install_outbound(
        &self,
        source_addr: Ipv4Addr,
        agent_leg_f_port: u16,
    ) -> InterceptResult<Box<dyn InterceptGuard>> {
        self.sim.install_outbound(source_addr, agent_leg_f_port)
    }

    fn install_inbound(
        &self,
        virt: SocketAddrV4,
        agent_leg_c_port: u16,
    ) -> InterceptResult<Box<dyn InterceptGuard>> {
        self.sim.install_inbound(virt, agent_leg_c_port)
    }

    fn observe_shared_state(&self) -> InterceptResult<Option<InterceptState>> {
        self.sim.observe_shared_state()
    }

    fn converge_allocation_elements(
        &self,
        expected: &InterceptMembers,
    ) -> InterceptResult<Option<InterceptState>> {
        self.sim.converge_allocation_elements(expected)
    }

    fn remove_allocation_elements(
        &self,
        source_addr: Ipv4Addr,
        destinations: &[SocketAddrV4],
    ) -> InterceptResult<InterceptState> {
        self.removals.lock().push(source_addr);
        if self.armed.load(Ordering::SeqCst) {
            return Err(injected_removal_error(source_addr));
        }
        self.sim.remove_allocation_elements(source_addr, destinations)
    }
}

// ---------------------------------------------------------------------------
// The pinned lease events, captured process-wide.
// ---------------------------------------------------------------------------

/// The pinned event a lease retirement emits.
const LEASE_RETIRED: &str = "guest_network.lease_retired";
/// The pinned event a lease release emits.
const LEASE_RELEASED: &str = "guest_network.lease_released";

/// One pinned lease event as production emitted it: its name and the
/// allocation its `alloc` field names (`None` when the event carried no
/// `alloc` field, which [`assert_every_lease_event_names_an_allocation`]
/// rejects).
#[derive(Debug, Clone, PartialEq, Eq)]
struct LeaseEvent {
    name: &'static str,
    alloc: Option<String>,
}

#[derive(Default)]
struct EventFields(BTreeMap<String, String>);

impl tracing::field::Visit for EventFields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}").trim_matches('"').to_owned());
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

/// Appends every `guest_network.lease_retired` / `lease_released` event, with
/// its `alloc` field, to the process-wide log.
struct LeaseEventLayer(Arc<Mutex<Vec<LeaseEvent>>>);

impl<S: Subscriber> Layer<S> for LeaseEventLayer {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let name = match event.metadata().name() {
            LEASE_RETIRED => LEASE_RETIRED,
            LEASE_RELEASED => LEASE_RELEASED,
            _ => return,
        };
        let mut fields = EventFields::default();
        event.record(&mut fields);
        let alloc = fields.0.remove("alloc");
        self.0.lock().push(LeaseEvent { name, alloc });
    }
}

/// The process-wide, append-only lease-event log, installed as the process's
/// global subscriber on first use. Call it before boot so no event is missed.
fn lease_event_log() -> &'static Arc<Mutex<Vec<LeaseEvent>>> {
    static LOG: OnceLock<Arc<Mutex<Vec<LeaseEvent>>>> = OnceLock::new();
    LOG.get_or_init(|| {
        let log = Arc::new(Mutex::new(Vec::new()));
        tracing::subscriber::set_global_default(
            tracing_subscriber::registry().with(LeaseEventLayer(Arc::clone(&log))),
        )
        .expect(
            "the lease-event capture is this test process's one global subscriber \
             (nextest runs one test per process)",
        );
        log
    })
}

/// The lease events naming `alloc`, in emission order.
fn lease_events_for(alloc: &str) -> Vec<&'static str> {
    lease_event_log()
        .lock()
        .iter()
        .filter(|event| event.alloc.as_deref() == Some(alloc))
        .map(|event| event.name)
        .collect()
}

/// Every captured lease event carries the pinned `alloc` field.
fn assert_every_lease_event_names_an_allocation() {
    let log = lease_event_log().lock().clone();
    assert!(
        log.iter().all(|event| event.alloc.is_some()),
        "every pinned lease event names its allocation: {log:?}"
    );
}

// ---------------------------------------------------------------------------
// One node composed through the production entry.
// ---------------------------------------------------------------------------

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

struct Node {
    handle: ServerHandle,
    clock: Arc<SimClock>,
    driver: Arc<SimDriver>,
    intercept: Arc<RemovalFaultIntercept>,
    client: reqwest::Client,
    base: String,
    _dir: TempDir,
}

impl Node {
    async fn boot() -> Self {
        let dir = TempDir::new().expect("tempdir");
        let data_dir = dir.path().join("data");
        let operator_config_dir = dir.path().join("conf");
        std::fs::create_dir_all(&data_dir).expect("create data dir");
        std::fs::create_dir_all(&operator_config_dir).expect("create operator config dir");
        let clock = Arc::new(SimClock::new());
        let clock_port: Arc<dyn Clock> = clock.clone();
        let intercept = Arc::new(RemovalFaultIntercept::new());
        let intercept_port: Arc<dyn MtlsIntercept> = intercept.clone();
        let guest_dns: Arc<dyn GuestDnsFactory> = Arc::new(SimGuestDnsFactory::default());
        let config = ServerConfig {
            bind: "127.0.0.1:0".parse().expect("parse bind addr"),
            data_dir,
            operator_config_dir: operator_config_dir.clone(),
            tick_cadence: TICK,
            clock: Arc::clone(&clock_port),
            dataplane: Some(super::dataplane_lo::lo_dataplane_config()),
            dataplane_override: Some(Arc::new(SimDataplane::new())),
            ..ServerConfig::new(Arc::new(SimKek::for_boot()), intercept_port, guest_dns)
        };
        let obs: Arc<dyn ObservationStore> =
            Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
        let driver = Arc::new(SimDriver::with_clock(DriverType::Vm, Arc::clone(&clock_port)));
        let driver_port: Arc<dyn Driver> = driver.clone();
        let wiring = GuestNetworkExecWiring::new(Arc::clone(&clock_port));
        let handle = run_server_with_obs_and_driver(
            config,
            obs,
            driver_port,
            Arc::new(SimVmHostState::new()),
            Arc::new(SimSharedGuestNetworkOwner::default()),
            wiring,
            CgroupManager::new(PathBuf::from("/sys/fs/cgroup"), Arc::new(SimCgroupFs::new())),
        )
        .await
        .expect("production boot");
        let bound = handle.local_addr().await.expect("server bound address");
        let client = client_trusting(&read_ca_from_trust_triple(&operator_config_dir));
        let base = format!("https://localhost:{}", bound.port());
        Self { handle, clock, driver, intercept, client, base, _dir: dir }
    }

    async fn submit(&self, spec: SubmitSpecInput) {
        let response = self
            .client
            .post(format!("{}/v1/workloads", self.base))
            .json(&SubmitWorkloadRequest { spec })
            .send()
            .await
            .expect("submit reaches the public API");
        assert!(response.status().is_success(), "submit: {response:?}");
    }

    async fn stop(&self, workload: &str) {
        let response = self
            .client
            .post(format!("{}/v1/workloads/{workload}/stop", self.base))
            .send()
            .await
            .expect("stop reaches the public API");
        assert!(response.status().is_success(), "stop {workload}: {response:?}");
    }

    async fn allocs(&self, workload: &str) -> AllocStatusResponse {
        let response = self
            .client
            .get(format!("{}/v1/allocs?job={workload}", self.base))
            .send()
            .await
            .expect("GET /v1/allocs reaches the public API");
        assert!(response.status().is_success(), "GET /v1/allocs?job={workload}: {response:?}");
        response.json().await.expect("AllocStatusResponse body")
    }

    /// Advance the injected clock tick by tick until `done` holds for the
    /// workload's allocation listing, returning that listing.
    async fn tick_until(
        &self,
        workload: &str,
        what: &str,
        done: impl Fn(&AllocStatusResponse) -> bool,
    ) -> AllocStatusResponse {
        for _ in 0..MAX_TICKS {
            let listing = self.allocs(workload).await;
            if done(&listing) {
                return listing;
            }
            advance_and_settle(&self.clock, TICK).await;
        }
        panic!(
            "{what}: not reached within {MAX_TICKS} ticks; last {:?}",
            self.allocs(workload).await
        );
    }

    /// Graceful shutdown while the injected clock keeps advancing, so no
    /// clock-bounded wait inside shutdown parks forever.
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

fn row<'a>(listing: &'a AllocStatusResponse, alloc: &str) -> Option<&'a AllocStatusRowBody> {
    listing.rows.iter().find(|row| row.alloc_id == alloc)
}

fn single_running(listing: &AllocStatusResponse) -> Option<String> {
    match listing.rows.as_slice() {
        [row] if row.state == AllocStateWire::Running => Some(row.alloc_id.clone()),
        _ => None,
    }
}

fn vm_service(id: &str) -> SubmitSpecInput {
    SubmitSpecInput::Service(ServiceSpecInput {
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
    })
}

fn vm_job(id: &str) -> SubmitSpecInput {
    SubmitSpecInput::Job(JobSpecInput {
        id: id.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 134_217_728 },
        driver: DriverInput::Vm(VmInput {
            command: "/bin/sleep".to_owned(),
            args: vec!["3600".to_owned()],
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
    })
}

// ---------------------------------------------------------------------------
// Scenarios.
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-SHARED-SWITCH
/// S-ND295-59 — Describe reports a stuck stop as cleanup-pending and never counts it as a running replica
/// CONTRACT_SHAPE: bounded-change.
///
/// A Service replica whose stop cannot remove its protection members keeps its
/// `Running` row (R10), is reported `network_cleanup_pending: true`, and is
/// excluded from `replicas_running`; once the removal succeeds on a retry the
/// allocation is `Terminated` and no longer pending.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER step 07-04 (S-ND295-59)"]
async fn a_stuck_stop_is_reported_cleanup_pending_and_is_not_a_running_replica() {
    let node = Node::boot().await;
    node.submit(vm_service("cleanup-svc")).await;
    let running = node
        .tick_until("cleanup-svc", "the Service replica reaches Running", |listing| {
            single_running(listing).is_some()
        })
        .await;
    let alloc = single_running(&running).expect("one Running replica");
    let before = row(&running, &alloc).expect("the replica row");
    assert!(
        !before.network_cleanup_pending,
        "a healthy replica is not cleanup-pending: {before:?}"
    );
    assert_eq!(running.replicas_running, 1, "the healthy replica counts as running: {running:?}");

    // WHEN the stop cannot remove the replica's protection members.
    node.intercept.arm(true);
    node.stop("cleanup-svc").await;
    let stuck = node
        .tick_until("cleanup-svc", "the stop attempts member removal", |_| {
            node.intercept.removal_calls() > 0
        })
        .await;

    // THEN the allocation is reported pending, keeps its Running row, and is
    // not a running replica.
    let stuck_row = row(&stuck, &alloc).expect("the stuck replica row");
    assert_eq!(
        stuck_row.state,
        AllocStateWire::Running,
        "a failed stop leaves the row Running (R10)"
    );
    assert!(
        stuck_row.network_cleanup_pending,
        "a stop whose protection removal failed is cleanup-pending: {stuck_row:?}"
    );
    assert_eq!(
        stuck.replicas_running, 0,
        "a cleanup-pending allocation is not a running replica: {stuck:?}"
    );

    // AND once a retried removal succeeds the allocation is Terminated and no
    // longer pending.
    node.intercept.arm(false);
    let settled = node
        .tick_until("cleanup-svc", "the retried stop terminates the replica", |listing| {
            row(listing, &alloc).is_some_and(|row| row.state == AllocStateWire::Terminated)
        })
        .await;
    let settled_row = row(&settled, &alloc).expect("the terminated replica row");
    assert!(
        !settled_row.network_cleanup_pending,
        "a Terminated allocation whose cleanup finished is not pending: {settled_row:?}"
    );
    assert_eq!(settled.replicas_running, 0, "a stopped Service runs no replica: {settled:?}");
    node.shutdown().await;
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH
/// S-ND295-59 — Describe reports a stuck stop as cleanup-pending and never counts it as a running replica
/// CONTRACT_SHAPE: bounded-change.
///
/// A crashed allocation (a `Failed` row whose lease is still admitted, before
/// any cleanup begins) and an allocation being reclaimed (a `Failed` row whose
/// lease is Retiring because its reclaim cannot remove its protection members)
/// are both reported `network_cleanup_pending: true`; once the reclaim succeeds
/// the lease is released and the allocation is no longer pending.
///
/// The two pending states are told apart by the pinned lease events, never by
/// the row: the crash state has no `guest_network.lease_retired` for the
/// allocation (its lease is admitted), and the reclaim in progress has at
/// least one `lease_retired` and no `lease_released` (its lease is Retiring)
/// while the row reports pending. Whether a retry's `retire` of an
/// already-Retiring lease emits `lease_retired` again is not pinned, so the
/// retired events are not counted exactly; the release is.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER step 07-04 (S-ND295-59)"]
async fn crashed_and_reclaiming_allocations_are_reported_cleanup_pending() {
    // Installed before boot so no lease event is missed.
    lease_event_log();
    let node = Node::boot().await;
    node.submit(vm_job("cleanup-job")).await;
    let running = node
        .tick_until("cleanup-job", "the Job allocation reaches Running", |listing| {
            single_running(listing).is_some()
        })
        .await;
    let alloc = single_running(&running).expect("one Running allocation");
    let alloc_id = AllocationId::new(&alloc).expect("wire alloc id parses");

    // WHEN the VM crashes. The clock is not advanced, so no reconcile begins
    // any cleanup: the row is Failed and the lease is still admitted.
    node.driver.inject_exit_after(
        &alloc_id,
        Duration::ZERO,
        ExitKind::Crashed { exit_code: Some(1), signal: None },
    );
    let mut crashed = None;
    for _ in 0..200 {
        let listing = node.allocs("cleanup-job").await;
        if row(&listing, &alloc).is_some_and(|row| row.state == AllocStateWire::Failed) {
            crashed = Some(listing);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let crashed = crashed.expect("the exit observer records the crash as a Failed row");
    let crashed_row = row(&crashed, &alloc).expect("the crashed row");
    assert!(
        crashed_row.network_cleanup_pending,
        "a crashed allocation awaiting cleanup (Failed, admitted lease) is pending: {crashed_row:?}"
    );
    assert_eq!(
        lease_events_for(&alloc),
        Vec::<&str>::new(),
        "before any cleanup begins the crashed allocation's lease is admitted: no lease event \
         names it"
    );

    // AND WHEN the workload is stopped while its reclaim cannot remove the
    // protection members, the allocation is being reclaimed: its lease is
    // retired before the removal is attempted, and the failed removal keeps it
    // Retiring.
    node.intercept.arm(true);
    node.stop("cleanup-job").await;
    let reclaiming = node
        .tick_until("cleanup-job", "the reclaim attempts member removal", |_| {
            node.intercept.removal_calls() > 0
        })
        .await;
    let reclaiming_row = row(&reclaiming, &alloc).expect("the reclaiming row");
    assert_eq!(
        reclaiming_row.state,
        AllocStateWire::Failed,
        "a reclaim writes no row: {reclaiming_row:?}"
    );
    let retiring = lease_events_for(&alloc);
    assert!(
        !retiring.is_empty() && retiring.iter().all(|name| *name == LEASE_RETIRED),
        "the reclaim in progress holds the lease Retiring: lease_retired and no \
         lease_released; got {retiring:?}"
    );
    assert!(
        reclaiming_row.network_cleanup_pending,
        "an allocation being reclaimed (Retiring lease) is pending: {reclaiming_row:?}"
    );

    // THEN once a reclaim succeeds the lease is released and nothing is pending.
    node.intercept.arm(false);
    let released = node
        .tick_until("cleanup-job", "a retried reclaim releases the lease", |listing| {
            row(listing, &alloc).is_some_and(|row| !row.network_cleanup_pending)
        })
        .await;
    assert!(
        row(&released, &alloc).is_some_and(|row| !row.network_cleanup_pending),
        "a released allocation is not pending: {released:?}"
    );
    // FD pins that `retire` emits `lease_retired`, not whether a retry's
    // `retire` of the already-Retiring lease emits it again, so only the
    // release is counted exactly.
    let settled = lease_events_for(&alloc);
    assert!(
        settled.len() >= 2
            && settled.last() == Some(&LEASE_RELEASED)
            && settled[..settled.len() - 1].iter().all(|name| *name == LEASE_RETIRED),
        "the successful reclaim releases the Retiring lease exactly once, last; got {settled:?}"
    );
    assert_every_lease_event_names_an_allocation();
    node.shutdown().await;
}
