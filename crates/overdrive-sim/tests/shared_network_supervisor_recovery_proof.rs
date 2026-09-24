//! Correctness-recovery proof §3.3 — complete shared-network supervisor
//! recovery (GH #295, `.context/netns-density-295-correctness-recovery-plan.md`,
//! correctness gap 2).
//!
//! Seeded Tier-1 simulation over the real production composition root
//! (`run_server_with_obs_and_driver`), the same injected-driver helper the
//! accepted S-ND295-13 invariant and the S-ND295-33 conformance lane use. Every
//! driven port is a Sim adapter. Faults enter only as owner-port outcomes of the
//! accepted `SimSharedGuestNetworkOwner` (ADR-0124: "the public sim owner
//! provides independent standing slots for all twelve components ... for
//! seeded owner-port schedules"), plus two test-local owner-adapter faults: a
//! latch that holds one audit call in flight, and an audit call that panics.
//! The retained production supervisor must author every
//! recovery transition, quiescence, retry, kill, and fail-stop: no gate
//! transition, observation row, or shutdown request is fabricated.
//!
//! The oracle is the accepted contract (ADR-0124; feature-delta RUN-295-B,
//! "EXEC-close linearization", the `SharedGuestNetworkOwner` port contract;
//! S-ND295-29/30/32). It is NOT fitted to current behaviour. Every cell prints
//! its seed and verdict append-only; a test fails when any cell is RED or
//! UNREACHED (a clause whose precondition the supervisor never produced).
//!
//! Reproduce: `OVERDRIVE_SUPERVISOR_PROOF_SEEDS=<seed>[,<seed>...]`.
//!
//! Admission (new guest-command release) is observed at the one boundary the
//! VM driver uses: a single non-blocking `GuestNetworkExecGate::claim_release`
//! poll (Open → claim, BootClosed/Recovering → pending, FailStop → `None`).
//! Recovery progress is read through the read-only `recovery_progress`
//! projection. Neither observation mutates supervisor state.
//!
//! Clauses: C0 healthy preservation; C1 one-second audit cadence; C2 detection
//! and admission closure for all twelve components; C3 component-specific TAP
//! quiescence; C4 exact-owner repair cadence, partial repair, single reopen;
//! C5 deadline fail-stop; C6 unconfirmed quiescence → affected-VM kill before
//! repair, then fail-stop; C7 late success (healed owner, in-flight read-back)
//! cannot reopen; C8 supervisor task loss observed immediately with the
//! accepted snapshot.
//!
//! Scope limit: listener (leg-F/leg-C) and DNS task-loss classes run through
//! the worker's private Tokio task owner and the DNS serve-task owner (ADR-0124:
//! "no test constructs those consequences"). The production composition root
//! composes both only on the real-dataplane path, whose probes bind real host
//! sockets and nft state; this in-process proof does not exercise them.
#![cfg(feature = "integration-tests")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stderr,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::large_futures,
    clippy::cast_possible_truncation,
    clippy::unchecked_time_subtraction,
    reason = "proof test: seed-bearing diagnostics, exact CONTRACT_SHAPE lines, and long narrative episodes; every Duration subtraction is `later_snapshot - earlier_snapshot` of the monotonically advancing simulated elapsed counter"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use base64::Engine;
use futures::FutureExt;
use overdrive_control_plane::api::{SubmitWorkloadRequest, SubmitWorkloadResponse};
use overdrive_control_plane::dataplane_config::DataplaneConfig;
use overdrive_control_plane::guest_network::{
    GuestNetworkOperation, GuestNetworkPlan, GuestNetworkProvisioner, Result as GuestNetworkResult,
    SharedGuestNetworkAuditError, SharedGuestNetworkOwner,
};
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server_with_obs_and_driver};
use overdrive_core::aggregate::{DriverInput, JobSpecInput, ResourcesInput, VmInput};
use overdrive_core::api::submit::SubmitSpecInput;
use overdrive_core::guest_network::{
    GuestNetworkExecGate, GuestNetworkExecSupervisor, GuestNetworkExecWiring, ServeShutdownRequest,
    SharedGuestNetworkComponent, SharedGuestNetworkFailStop, SharedGuestNetworkFailStopCause,
    SharedGuestNetworkRecovery,
};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use overdrive_sim::adapters::SimKek;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::vm_host_state::SimVmHostState;
use parking_lot::Mutex;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tokio::sync::Semaphore;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

// ---------------------------------------------------------------------------
// Accepted contract constants (ADR-0124 / RUN-295-B).
// ---------------------------------------------------------------------------

/// One-second kernel/owner audit period.
const AUDIT_PERIOD: Duration = Duration::from_secs(1);
/// Exact-owner retry cadence.
const ATTEMPT_PERIOD: Duration = Duration::from_millis(250);
/// Bounded recovery window before the typed fail-stop request.
const RECOVERY_DEADLINE: Duration = Duration::from_secs(5);
/// Completed attempts at the recovery deadline.
const DEADLINE_ATTEMPTS: u32 = 20;
/// Injected-clock step between observations.
const STEP: Duration = Duration::from_millis(10);

/// The closed component vocabulary in the accepted audit order.
const COMPONENTS: [SharedGuestNetworkComponent; 12] = [
    SharedGuestNetworkComponent::Bridge,
    SharedGuestNetworkComponent::LegF,
    SharedGuestNetworkComponent::LegC,
    SharedGuestNetworkComponent::Dns,
    SharedGuestNetworkComponent::TcxLink,
    SharedGuestNetworkComponent::EndpointMap,
    SharedGuestNetworkComponent::CounterMap,
    SharedGuestNetworkComponent::BpffsPin,
    SharedGuestNetworkComponent::BridgeGuard,
    SharedGuestNetworkComponent::IpRules,
    SharedGuestNetworkComponent::IpSets,
    SharedGuestNetworkComponent::Supervisor,
];

/// Which production owner repairs a component (ADR-0124: bridge/TAP/TCX/map/
/// pin/bridge-guard ownership is the one `SharedGuestNetworkOwner`; listeners
/// and the constant IP rule/set program belong to the mTLS worker; DNS to the
/// DNS serve-task owner).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepairOwner {
    SharedGuestNetworkOwner,
    MtlsWorker,
    DnsOwner,
    NoOwner,
}

const fn repair_owner(component: SharedGuestNetworkComponent) -> RepairOwner {
    match component {
        SharedGuestNetworkComponent::Bridge
        | SharedGuestNetworkComponent::TcxLink
        | SharedGuestNetworkComponent::EndpointMap
        | SharedGuestNetworkComponent::CounterMap
        | SharedGuestNetworkComponent::BpffsPin
        | SharedGuestNetworkComponent::BridgeGuard => RepairOwner::SharedGuestNetworkOwner,
        SharedGuestNetworkComponent::LegF
        | SharedGuestNetworkComponent::LegC
        | SharedGuestNetworkComponent::IpRules
        | SharedGuestNetworkComponent::IpSets => RepairOwner::MtlsWorker,
        SharedGuestNetworkComponent::Dns => RepairOwner::DnsOwner,
        SharedGuestNetworkComponent::Supervisor => RepairOwner::NoOwner,
    }
}

/// Component-specific TAP quiescence rule.
///
/// ADR-0124: "Kernel-path mismatch also quiesces managed TAPs" and "A pure
/// listener failure does not quiesce TAPs or existing commands". RUN-295-B:
/// every bridge/TCX/map/pin/nft row quiesces; "a pure listener/DNS task exit
/// does not require TAP-down" and the DNS row carries no quiesce. The
/// `Supervisor` component has no accepted quiescence rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuiesceRule {
    ExactlyOnceBeforeRepair,
    Never,
    Unspecified,
}

const fn quiesce_rule(component: SharedGuestNetworkComponent) -> QuiesceRule {
    match component {
        SharedGuestNetworkComponent::Bridge
        | SharedGuestNetworkComponent::TcxLink
        | SharedGuestNetworkComponent::EndpointMap
        | SharedGuestNetworkComponent::CounterMap
        | SharedGuestNetworkComponent::BpffsPin
        | SharedGuestNetworkComponent::BridgeGuard
        | SharedGuestNetworkComponent::IpRules
        | SharedGuestNetworkComponent::IpSets => QuiesceRule::ExactlyOnceBeforeRepair,
        SharedGuestNetworkComponent::LegF
        | SharedGuestNetworkComponent::LegC
        | SharedGuestNetworkComponent::Dns => QuiesceRule::Never,
        SharedGuestNetworkComponent::Supervisor => QuiesceRule::Unspecified,
    }
}

fn kernel_path_components() -> Vec<SharedGuestNetworkComponent> {
    COMPONENTS
        .into_iter()
        .filter(|component| quiesce_rule(*component) == QuiesceRule::ExactlyOnceBeforeRepair)
        .collect()
}

// ---------------------------------------------------------------------------
// Seeds.
// ---------------------------------------------------------------------------

const DEFAULT_SEEDS: [u64; 2] = [0x2953_3000_0000_0001, 0x2953_3000_5eed_0002];

fn seeds() -> Vec<u64> {
    std::env::var("OVERDRIVE_SUPERVISOR_PROOF_SEEDS").map_or_else(
        |_| DEFAULT_SEEDS.to_vec(),
        |raw| {
            raw.split(',')
                .map(|part| {
                    let part = part.trim();
                    part.strip_prefix("0x")
                        .map_or_else(|| part.parse::<u64>(), |hex| u64::from_str_radix(hex, 16))
                        .unwrap_or_else(|error| panic!("invalid proof seed {part:?}: {error}"))
                })
                .collect()
        },
    )
}

fn cell_rng(seed: u64, cell: usize) -> StdRng {
    StdRng::seed_from_u64(seed ^ (cell as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// A seeded offset, a whole number of steps inside one audit period.
fn seeded_phase(rng: &mut StdRng) -> Duration {
    STEP * rng.gen_range(0..100_u32)
}

// ---------------------------------------------------------------------------
// Append-only operational-event capture (guest_network.shared_owner_*).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct OwnerEvent {
    name: String,
    fields: BTreeMap<String, String>,
}

#[derive(Default)]
struct FieldVisitor(BTreeMap<String, String>);

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }
}

struct OwnerEventCapture {
    store: Arc<Mutex<Vec<OwnerEvent>>>,
}

impl<S> Layer<S> for OwnerEventCapture
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let name = event.metadata().name();
        if !name.starts_with("guest_network.shared_owner") {
            return;
        }
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.store.lock().push(OwnerEvent { name: name.to_owned(), fields: visitor.0 });
    }
}

fn event_store() -> &'static Arc<Mutex<Vec<OwnerEvent>>> {
    static STORE: OnceLock<Arc<Mutex<Vec<OwnerEvent>>>> = OnceLock::new();
    STORE.get_or_init(|| {
        let store = Arc::new(Mutex::new(Vec::new()));
        let capture =
            OwnerEventCapture { store: Arc::clone(&store) }.with_filter(LevelFilter::INFO);
        tracing::subscriber::set_global_default(Registry::default().with(capture))
            .expect("proof binary installs exactly one global capture subscriber");
        store
    })
}

fn events_since(cursor: usize) -> Vec<OwnerEvent> {
    event_store().lock()[cursor..].to_vec()
}

fn events_cursor() -> usize {
    event_store().lock().len()
}

fn named<'a>(events: &'a [OwnerEvent], name: &str) -> Vec<&'a OwnerEvent> {
    events.iter().filter(|event| event.name == name).collect()
}

// ---------------------------------------------------------------------------
// Test-local owner-port double: the accepted Sim owner plus one audit latch.
// ---------------------------------------------------------------------------

/// Delegates every port call to the accepted `SimSharedGuestNetworkOwner`.
/// Two driven-port faults are layered on top, neither of which authors a
/// supervisor consequence:
///
/// - when the latch is armed, `audit_shared` awaits one permit before
///   delegating, modelling one owner read-back still in flight (a hung host
///   call);
/// - when the panic slot is armed, `audit_shared` panics, modelling an owner
///   adapter defect that unwinds the task polling it. The supervisor task-loss
///   classification (`SupervisorPanicked`) is authored by production.
#[derive(Default)]
struct ProofOwner {
    sim: SimSharedGuestNetworkOwner,
    audit_latch: Mutex<Option<Arc<Semaphore>>>,
    audits_in_flight: AtomicUsize,
    audit_panic: AtomicBool,
    audit_panics: AtomicUsize,
}

impl ProofOwner {
    fn arm_audit_latch(&self) -> Arc<Semaphore> {
        let latch = Arc::new(Semaphore::new(0));
        *self.audit_latch.lock() = Some(Arc::clone(&latch));
        latch
    }

    fn disarm_audit_latch(&self) {
        *self.audit_latch.lock() = None;
    }

    fn audits_in_flight(&self) -> usize {
        self.audits_in_flight.load(Ordering::SeqCst)
    }

    fn arm_audit_panic(&self) {
        self.audit_panic.store(true, Ordering::SeqCst);
    }

    fn audit_panics(&self) -> usize {
        self.audit_panics.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl GuestNetworkProvisioner for ProofOwner {
    async fn provision(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        self.sim.provision(plan).await
    }
    async fn activate(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        self.sim.activate(plan).await
    }
    async fn teardown(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        self.sim.teardown(plan).await
    }
}

#[async_trait::async_trait]
impl SharedGuestNetworkOwner for ProofOwner {
    async fn probe_startup(&self) -> GuestNetworkResult<()> {
        self.sim.probe_startup().await
    }
    async fn sweep_stale(&self) -> GuestNetworkResult<()> {
        self.sim.sweep_stale().await
    }
    async fn converge_shared(&self) -> GuestNetworkResult<()> {
        self.sim.converge_shared().await
    }
    async fn audit_shared(&self) -> std::result::Result<(), SharedGuestNetworkAuditError> {
        if self.audit_panic.swap(false, Ordering::SeqCst) {
            self.audit_panics.fetch_add(1, Ordering::SeqCst);
            panic!("proof owner-port fault: shared-owner audit adapter panicked");
        }
        let latch = self.audit_latch.lock().clone();
        if let Some(latch) = latch {
            self.audits_in_flight.fetch_add(1, Ordering::SeqCst);
            latch.acquire().await.expect("proof audit latch is never closed").forget();
            self.audits_in_flight.fetch_sub(1, Ordering::SeqCst);
        }
        self.sim.audit_shared().await
    }
    async fn quiesce_managed_taps(&self) -> GuestNetworkResult<()> {
        self.sim.quiesce_managed_taps().await
    }
}

// ---------------------------------------------------------------------------
// One production node composed through the real composition root.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admission {
    Open,
    Closed,
    FailStopped,
}

struct Node {
    seed: u64,
    handle: Option<ServerHandle>,
    request: Option<(Duration, ServeShutdownRequest)>,
    clock: Arc<SimClock>,
    owner: Arc<ProofOwner>,
    gate: Arc<GuestNetworkExecGate>,
    exec: Arc<GuestNetworkExecSupervisor>,
    obs: Arc<SimObservationStore>,
    config_dir: PathBuf,
    elapsed: Duration,
    boot_calls: usize,
    _dir: tempfile::TempDir,
}

async fn settle() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

/// Real-I/O settle for HTTPS/redb phases only (workload deploy). Wall-clock
/// here is integration-host scheduling, never a simulated measurement.
async fn settle_io() {
    tokio::time::sleep(Duration::from_millis(5)).await;
    settle().await;
}

impl Node {
    async fn boot(seed: u64) -> Self {
        let _ = event_store();
        let dir = tempfile::tempdir().expect("proof tempdir");
        let data_dir = dir.path().join("data");
        let config_dir = dir.path().join("operator");
        std::fs::create_dir_all(&data_dir).expect("proof data dir");
        std::fs::create_dir_all(&config_dir).expect("proof operator dir");
        let clock = Arc::new(SimClock::new());
        let clock_port: Arc<dyn Clock> = clock.clone();
        let config = ServerConfig {
            bind: "127.0.0.1:0".parse().expect("loopback bind"),
            data_dir,
            operator_config_dir: config_dir.clone(),
            clock: Arc::clone(&clock_port),
            dataplane: Some(DataplaneConfig {
                client_iface: "lo".to_owned(),
                backend_iface: "lo".to_owned(),
            }),
            dataplane_override: Some(Arc::new(SimDataplane::new())),
            ..ServerConfig::new(Arc::new(SimKek::for_boot()))
        };
        let owner = Arc::new(ProofOwner::default());
        let owner_port: Arc<dyn SharedGuestNetworkOwner> = owner.clone();
        let obs = Arc::new(SimObservationStore::single_peer(
            NodeId::new("nd295-proof-3-3").expect("node id"),
            seed,
        ));
        let obs_port: Arc<dyn ObservationStore> = obs.clone();
        let driver: Arc<dyn Driver> =
            Arc::new(SimDriver::with_clock(DriverType::Vm, Arc::clone(&clock_port)));
        let wiring = GuestNetworkExecWiring::new(Arc::clone(&clock_port));
        let gate = wiring.gate();
        let exec = wiring.supervisor();
        let handle = run_server_with_obs_and_driver(
            config,
            obs_port,
            driver,
            Arc::new(SimVmHostState::new()),
            owner_port,
            wiring,
        )
        .await
        .unwrap_or_else(|error| panic!("seed={seed:#x}: production boot failed: {error}"));
        settle().await;
        let boot_calls = owner.sim.calls().len();
        eprintln!("seed={seed:#x} diagnostic=boot_owner_journal calls={:?}", owner.sim.calls());
        Self {
            seed,
            handle: Some(handle),
            request: None,
            clock,
            owner,
            gate,
            exec,
            obs,
            config_dir,
            elapsed: Duration::ZERO,
            boot_calls,
            _dir: dir,
        }
    }

    /// One non-blocking release attempt at the VM driver's claim boundary.
    fn admission(&self) -> Admission {
        match self.gate.claim_release().now_or_never() {
            Some(Some(claim)) => {
                drop(claim);
                Admission::Open
            }
            Some(None) => Admission::FailStopped,
            None => Admission::Closed,
        }
    }

    fn progress(&self) -> Option<SharedGuestNetworkRecovery> {
        self.exec.recovery_progress()
    }

    /// Owner-port calls issued after boot completed.
    fn runtime_calls(&self) -> Vec<GuestNetworkOperation> {
        self.owner.sim.calls()[self.boot_calls..].to_vec()
    }

    fn call_count(&self) -> usize {
        self.owner.sim.calls().len() - self.boot_calls
    }

    /// Observe the typed shutdown request without consuming the handle. Once a
    /// request is observed it is retained and never polled for again.
    fn poll_request(&mut self) -> Option<&ServeShutdownRequest> {
        if self.request.is_none() {
            let handle = self.handle.as_mut().expect("live server handle");
            if let Some(request) = handle.shutdown_requested().now_or_never() {
                self.request = Some((self.elapsed, request));
            }
        }
        self.request.as_ref().map(|(_, request)| request)
    }

    async fn step(&mut self) {
        self.clock.tick(STEP);
        self.elapsed += STEP;
        settle().await;
        let _ = self.poll_request();
    }

    async fn advance(&mut self, duration: Duration) {
        let target = self.elapsed + duration;
        while self.elapsed < target {
            self.step().await;
        }
    }

    async fn shutdown(mut self) {
        let seed = self.seed;
        if let Some(handle) = self.handle.take()
            && let Err(error) = handle.shutdown(Duration::from_secs(2)).await
        {
            eprintln!("seed={seed:#x} diagnostic=node_shutdown_error error={error}");
        }
    }
}

fn count(calls: &[GuestNetworkOperation], operation: GuestNetworkOperation) -> usize {
    calls.iter().filter(|call| **call == operation).count()
}

// ---------------------------------------------------------------------------
// Verdict bookkeeping.
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum Verdict {
    Green,
    Red(String),
    Unreached(String),
}

struct Report {
    clause: &'static str,
    cells: Vec<(u64, String, Verdict)>,
}

impl Report {
    fn new(clause: &'static str) -> Self {
        eprintln!("proof=3.3 clause={clause} seeds={:x?}", seeds());
        Self { clause, cells: Vec::new() }
    }

    fn record(&mut self, seed: u64, cell: impl Into<String>, verdict: Verdict) {
        let cell = cell.into();
        let (label, detail) = match &verdict {
            Verdict::Green => ("GREEN", String::new()),
            Verdict::Red(detail) => ("RED", detail.clone()),
            Verdict::Unreached(detail) => ("UNREACHED", detail.clone()),
        };
        eprintln!(
            "proof=3.3 clause={} seed={seed:#x} cell={cell} verdict={label} {detail}",
            self.clause
        );
        self.cells.push((seed, cell, verdict));
    }

    fn finish(self) {
        let total = self.cells.len();
        let red: Vec<_> = self
            .cells
            .iter()
            .filter(|(_, _, verdict)| matches!(verdict, Verdict::Red(_)))
            .collect();
        let unreached: Vec<_> = self
            .cells
            .iter()
            .filter(|(_, _, verdict)| matches!(verdict, Verdict::Unreached(_)))
            .collect();
        eprintln!(
            "proof=3.3 clause={} summary total={total} green={} red={} unreached={}",
            self.clause,
            total - red.len() - unreached.len(),
            red.len(),
            unreached.len()
        );
        if let Some((seed, cell, verdict)) = red.first().or_else(|| unreached.first()) {
            panic!(
                "clause {} violated in {} of {total} cells (red={}, unreached={}); first: seed={seed:#x} cell={cell} {verdict:?}",
                self.clause,
                red.len() + unreached.len(),
                red.len(),
                unreached.len()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Shared episode: arm one component at a seeded phase and await detection.
// ---------------------------------------------------------------------------

struct Detected {
    at: Duration,
    latency: Duration,
    progress: SharedGuestNetworkRecovery,
    admission: Admission,
    calls_at_arm: usize,
    events_at_arm: usize,
}

struct Missed {
    admission: Admission,
    audits_since_arm: usize,
    calls_since_arm: Vec<GuestNetworkOperation>,
}

impl std::fmt::Display for Missed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "no recovery within one audit period (+1 step) of the owner-port fault: admission={:?} audit_shared_calls_since_arm={} owner_calls_since_arm={:?}",
            self.admission, self.audits_since_arm, self.calls_since_arm
        )
    }
}

async fn arm_and_detect(
    node: &mut Node,
    component: SharedGuestNetworkComponent,
    phase: Duration,
) -> Result<Detected, Missed> {
    node.advance(phase).await;
    let armed_at = node.elapsed;
    let calls_at_arm = node.call_count();
    let events_at_arm = events_cursor();
    node.owner.sim.script_component_audit_failure(component, true);
    while node.elapsed - armed_at < AUDIT_PERIOD + STEP {
        node.step().await;
        if let Some(progress) = node.progress() {
            return Ok(Detected {
                at: node.elapsed,
                latency: node.elapsed - armed_at,
                progress,
                admission: node.admission(),
                calls_at_arm,
                events_at_arm,
            });
        }
    }
    let calls_since_arm = node.runtime_calls()[calls_at_arm..].to_vec();
    Err(Missed {
        admission: node.admission(),
        audits_since_arm: count(&calls_since_arm, GuestNetworkOperation::BridgeObserve),
        calls_since_arm,
    })
}

/// Advance until `progress.attempts` reaches `attempts`, returning the time it
/// was first observed, or `None` if the next attempt did not complete within
/// one attempt period (+1 step).
async fn await_attempt(
    node: &mut Node,
    attempts: u32,
) -> Option<(Duration, SharedGuestNetworkRecovery)> {
    let start = node.elapsed;
    while node.elapsed - start < ATTEMPT_PERIOD + STEP {
        node.step().await;
        match node.progress() {
            Some(progress) if progress.attempts >= attempts => {
                return Some((node.elapsed, progress));
            }
            None => return None,
            Some(_) => {}
        }
    }
    None
}

fn within_one_step(observed: Duration, expected: Duration) -> bool {
    observed.abs_diff(expected) <= STEP
}

// ---------------------------------------------------------------------------
// C0 — healthy control.
// ---------------------------------------------------------------------------

/// Control: a healthy shared owner never enters recovery, never quiesces or
/// repairs, and keeps admission open.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread")]
async fn healthy_owner_keeps_admission_open_without_recovery_effects() {
    let mut report = Report::new("C0-healthy-preservation");
    for seed in seeds() {
        let mut node = Node::boot(seed).await;
        let events_at_boot = events_cursor();
        let mut violation = (node.admission() != Admission::Open)
            .then(|| format!("admission after boot is {:?}", node.admission()));
        for _ in 0..1_000 {
            node.step().await;
            if violation.is_none() {
                if let Some(progress) = node.progress() {
                    violation =
                        Some(format!("spurious recovery at {:?}: {progress:?}", node.elapsed));
                } else if node.admission() != Admission::Open {
                    violation =
                        Some(format!("admission {:?} at {:?}", node.admission(), node.elapsed));
                } else if node.request.is_some() {
                    violation = Some(format!("spurious shutdown request {:?}", node.request));
                }
            }
        }
        let calls = node.runtime_calls();
        if violation.is_none() {
            for forbidden in
                [GuestNetworkOperation::TapSetDown, GuestNetworkOperation::BridgeConverge]
            {
                if count(&calls, forbidden) != 0 {
                    violation = Some(format!("healthy owner received {forbidden:?}: {calls:?}"));
                }
            }
        }
        let unhealthy =
            named(&events_since(events_at_boot), "guest_network.shared_owner_unhealthy").len();
        if violation.is_none() && unhealthy != 0 {
            violation = Some(format!("{unhealthy} unhealthy events on a healthy owner"));
        }
        report.record(seed, "healthy-10s", violation.map_or(Verdict::Green, Verdict::Red));
        node.shutdown().await;
    }
    report.finish();
}

// ---------------------------------------------------------------------------
// C1 — the one-second audit reads the shared owner.
// ---------------------------------------------------------------------------

/// The retained supervisor audits the complete registered shared-owner
/// inventory (`SharedGuestNetworkOwner::audit_shared`) every second.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn healthy_node_audits_the_shared_owner_every_second() {
    let mut report = Report::new("C1-one-second-audit-cadence");
    for seed in seeds() {
        let mut node = Node::boot(seed).await;
        let mut audit_times = Vec::new();
        let mut seen = 0;
        for _ in 0..1_000 {
            node.step().await;
            let audits = count(&node.runtime_calls(), GuestNetworkOperation::BridgeObserve);
            for _ in seen..audits {
                audit_times.push(node.elapsed);
            }
            seen = audits;
        }
        let window = node.elapsed;
        let mut gaps = Vec::new();
        let mut previous = Duration::ZERO;
        for at in &audit_times {
            gaps.push(*at - previous);
            previous = *at;
        }
        gaps.push(window - previous);
        let max_gap = gaps.iter().copied().max().unwrap_or(window);
        let verdict = if max_gap <= AUDIT_PERIOD + STEP {
            Verdict::Green
        } else {
            Verdict::Red(format!(
                "audit_shared calls in {window:?} = {}; largest unaudited interval {max_gap:?} > {AUDIT_PERIOD:?} (audit times {audit_times:?})",
                audit_times.len()
            ))
        };
        report.record(seed, "healthy-10s", verdict);
        node.shutdown().await;
    }
    report.finish();
}

// ---------------------------------------------------------------------------
// C2 — every component is detected within one audit and closes admission.
// ---------------------------------------------------------------------------

/// Every accepted component's owner-port loss is detected within one audit
/// period, closes new guest-command release, records `Recovering{component,0}`,
/// and emits exactly one unhealthy event naming the component.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn every_component_loss_is_detected_within_one_audit_and_closes_admission() {
    let mut report = Report::new("C2-detection-and-admission-closure");
    for seed in seeds() {
        for (index, component) in COMPONENTS.into_iter().enumerate() {
            let mut rng = cell_rng(seed, index);
            let phase = seeded_phase(&mut rng);
            let mut node = Node::boot(seed).await;
            let cell = format!("{component:?}@phase={}ms", phase.as_millis());
            let verdict = match arm_and_detect(&mut node, component, phase).await {
                Err(missed) => Verdict::Red(missed.to_string()),
                Ok(detected) => {
                    let events = events_since(detected.events_at_arm);
                    let unhealthy = named(&events, "guest_network.shared_owner_unhealthy");
                    if detected.progress.component != component || detected.progress.attempts != 0 {
                        Verdict::Red(format!("detection snapshot {:?}", detected.progress))
                    } else if detected.latency > AUDIT_PERIOD + STEP {
                        Verdict::Red(format!("detection latency {:?}", detected.latency))
                    } else if detected.admission != Admission::Closed {
                        Verdict::Red(format!("admission {:?} while Recovering", detected.admission))
                    } else if unhealthy.len() != 1
                        || unhealthy[0].fields.get("component") != Some(&format!("{component:?}"))
                    {
                        Verdict::Red(format!("unhealthy events {unhealthy:?}"))
                    } else if !unhealthy[0].fields.contains_key("cause") {
                        // RUN-295-B: `guest_network.shared_owner_unhealthy{component,cause}`.
                        Verdict::Red(format!("unhealthy event carries no cause: {unhealthy:?}"))
                    } else {
                        Verdict::Green
                    }
                }
            };
            report.record(seed, cell, verdict);
            node.shutdown().await;
        }
    }
    report.finish();
}

// ---------------------------------------------------------------------------
// C3 — component-specific TAP quiescence.
// ---------------------------------------------------------------------------

/// Kernel-path loss quiesces managed TAPs exactly once, before the first
/// repair attempt; pure listener or DNS loss never quiesces TAPs.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn kernel_path_loss_quiesces_once_before_repair_and_listener_or_dns_loss_never_does() {
    let mut report = Report::new("C3-component-quiescence-rules");
    for seed in seeds() {
        for (index, component) in COMPONENTS.into_iter().enumerate() {
            let rule = quiesce_rule(component);
            if rule == QuiesceRule::Unspecified {
                continue;
            }
            let mut rng = cell_rng(seed, index);
            let phase = seeded_phase(&mut rng);
            let failed_attempts = rng.gen_range(0..4_u32);
            let mut node = Node::boot(seed).await;
            let cell = format!("{component:?}/{rule:?}/failed_attempts={failed_attempts}");
            let verdict = match arm_and_detect(&mut node, component, phase).await {
                Err(missed) => Verdict::Unreached(format!("precondition detection: {missed}")),
                Ok(detected) => {
                    let mut unreached = None;
                    for attempt in 1..=failed_attempts {
                        if await_attempt(&mut node, attempt).await.is_none() {
                            unreached = Some(format!("attempt {attempt} never completed"));
                            break;
                        }
                    }
                    node.owner.sim.script_component_audit_failure(component, false);
                    if unreached.is_none() {
                        node.advance(ATTEMPT_PERIOD + STEP).await;
                    }
                    let episode = node.runtime_calls()[detected.calls_at_arm..].to_vec();
                    let quiesces = count(&episode, GuestNetworkOperation::TapSetDown);
                    let first_quiesce =
                        episode.iter().position(|call| *call == GuestNetworkOperation::TapSetDown);
                    let detection_audit = episode
                        .iter()
                        .position(|call| *call == GuestNetworkOperation::BridgeObserve);
                    let first_repair = episode
                        .iter()
                        .enumerate()
                        .skip(detection_audit.map_or(0, |index| index + 1))
                        .find(|(_, call)| {
                            matches!(
                                call,
                                GuestNetworkOperation::BridgeConverge
                                    | GuestNetworkOperation::BridgeObserve
                            )
                        })
                        .map(|(index, _)| index);
                    match (rule, unreached) {
                        (_, Some(reason)) => Verdict::Unreached(reason),
                        (QuiesceRule::ExactlyOnceBeforeRepair, None) => {
                            if quiesces != 1 {
                                Verdict::Red(format!("TapSetDown x{quiesces}: {episode:?}"))
                            } else if first_repair.is_some_and(|repair| {
                                first_quiesce.is_some_and(|quiesce| quiesce > repair)
                            }) {
                                Verdict::Red(format!("repair preceded quiesce: {episode:?}"))
                            } else {
                                Verdict::Green
                            }
                        }
                        (QuiesceRule::Never, None) => {
                            if quiesces == 0 {
                                Verdict::Green
                            } else {
                                Verdict::Red(format!(
                                    "pure listener/DNS loss quiesced TAPs x{quiesces}: {episode:?}"
                                ))
                            }
                        }
                        (QuiesceRule::Unspecified, None) => unreachable!("skipped above"),
                    }
                }
            };
            report.record(seed, cell, verdict);
            node.shutdown().await;
        }
    }
    report.finish();
}

// ---------------------------------------------------------------------------
// C4 — exact-owner repair on the attempt cadence; partial repair; one reopen.
// ---------------------------------------------------------------------------

/// Recovery retries the exact failed component through its owning component
/// every 250 ms; each attempt is counted only after convergence plus full
/// audit; partial repair records the first remaining component and never
/// reopens; complete repair reopens admission exactly once.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn repair_runs_through_the_owning_component_on_the_attempt_cadence_and_reopens_once() {
    let mut report = Report::new("C4-exact-owner-repair-cadence-reopen");
    for seed in seeds() {
        for (index, component) in COMPONENTS.into_iter().enumerate() {
            let mut rng = cell_rng(seed, index);
            let phase = seeded_phase(&mut rng);
            let failed_attempts = rng.gen_range(0..=12_u32);
            // Partial repair only within the shared-owner class, so the exact
            // owner of the first remaining component is unchanged.
            let partial = (repair_owner(component) == RepairOwner::SharedGuestNetworkOwner
                && rng.gen_bool(0.5))
            .then(|| {
                let peers: Vec<_> = COMPONENTS
                    .into_iter()
                    .filter(|peer| {
                        *peer != component
                            && repair_owner(*peer) == RepairOwner::SharedGuestNetworkOwner
                    })
                    .collect();
                (peers[rng.gen_range(0..peers.len())], rng.gen_range(1..=4_u32))
            });
            let mut node = Node::boot(seed).await;
            let cell = format!(
                "{component:?}/owner={:?}/failed_attempts={failed_attempts}/partial={partial:?}",
                repair_owner(component)
            );
            let verdict = match arm_and_detect(&mut node, component, phase).await {
                Err(missed) => Verdict::Unreached(format!("precondition detection: {missed}")),
                Ok(detected) => {
                    recovery_episode(&mut node, component, &detected, failed_attempts, partial)
                        .await
                }
            };
            report.record(seed, cell, verdict);
            node.shutdown().await;
        }
    }
    report.finish();
}

async fn recovery_episode(
    node: &mut Node,
    component: SharedGuestNetworkComponent,
    detected: &Detected,
    failed_attempts: u32,
    partial: Option<(SharedGuestNetworkComponent, u32)>,
) -> Verdict {
    let calls_at_detection = node.call_count();
    // Phase 1: `failed_attempts` failed attempts against the original component.
    for attempt in 1..=failed_attempts {
        let Some((at, progress)) = await_attempt(node, attempt).await else {
            return Verdict::Red(format!("attempt {attempt} did not complete within one period"));
        };
        let expected_elapsed = ATTEMPT_PERIOD * attempt;
        if progress
            != (SharedGuestNetworkRecovery {
                component,
                attempts: attempt,
                elapsed: progress.elapsed,
            })
            || !within_one_step(progress.elapsed, expected_elapsed)
            || !within_one_step(at - detected.at, expected_elapsed)
        {
            return Verdict::Red(format!(
                "attempt {attempt} snapshot {progress:?} at {:?} after detection",
                at - detected.at
            ));
        }
        if node.admission() != Admission::Closed {
            return Verdict::Red(format!(
                "admission {:?} during attempt {attempt}",
                node.admission()
            ));
        }
    }
    // Phase 2 (optional): the original component heals but a second remains.
    let mut total_failed = failed_attempts;
    let mut remaining = component;
    node.owner.sim.script_component_audit_failure(component, false);
    if let Some((second, partial_attempts)) = partial {
        node.owner.sim.script_component_audit_failure(second, true);
        remaining = second;
        for offset in 1..=partial_attempts {
            let attempt = failed_attempts + offset;
            let Some((_, progress)) = await_attempt(node, attempt).await else {
                return Verdict::Red(format!("partial attempt {attempt} did not complete"));
            };
            if progress.component != second || progress.attempts != attempt {
                return Verdict::Red(format!(
                    "partial repair snapshot {progress:?}; expected first remaining {second:?}"
                ));
            }
            if node.admission() != Admission::Closed {
                return Verdict::Red(format!(
                    "partial repair reopened admission at attempt {attempt}"
                ));
            }
        }
        total_failed += partial_attempts;
        node.owner.sim.script_component_audit_failure(second, false);
    }
    // Phase 3: the next attempt converges and passes the full audit.
    let reopen_deadline = node.elapsed + ATTEMPT_PERIOD + STEP;
    let mut reopened_at = None;
    while node.elapsed < reopen_deadline {
        node.step().await;
        if node.progress().is_none() {
            reopened_at = Some(node.elapsed);
            break;
        }
    }
    let Some(reopened_at) = reopened_at else {
        return Verdict::Red(format!(
            "no reopen one attempt after repair; progress {:?}",
            node.progress()
        ));
    };
    let expected_reopen = ATTEMPT_PERIOD * (total_failed + 1);
    if !within_one_step(reopened_at - detected.at, expected_reopen) {
        return Verdict::Red(format!(
            "reopened {:?} after detection; expected {expected_reopen:?}",
            reopened_at - detected.at
        ));
    }
    if node.admission() != Admission::Open {
        return Verdict::Red(format!("admission {:?} after complete repair", node.admission()));
    }
    let recovery_calls = node.runtime_calls()[calls_at_detection..].to_vec();
    let completed = total_failed + 1;
    let converges = count(&recovery_calls, GuestNetworkOperation::BridgeConverge) as u32;
    let audits = count(&recovery_calls, GuestNetworkOperation::BridgeObserve) as u32;
    match repair_owner(component) {
        RepairOwner::SharedGuestNetworkOwner => {
            if converges != completed {
                return Verdict::Red(format!(
                    "shared-owner converge x{converges} for {completed} attempts: {recovery_calls:?}"
                ));
            }
            let pairs = recovery_calls
                .iter()
                .filter(|call| {
                    matches!(
                        call,
                        GuestNetworkOperation::BridgeConverge
                            | GuestNetworkOperation::BridgeObserve
                    )
                })
                .copied()
                .collect::<Vec<_>>();
            let expected_pairs =
                [GuestNetworkOperation::BridgeConverge, GuestNetworkOperation::BridgeObserve]
                    .repeat(completed as usize);
            if pairs != expected_pairs {
                return Verdict::Red(format!(
                    "attempts are not converge-then-full-audit pairs: {recovery_calls:?}"
                ));
            }
        }
        RepairOwner::MtlsWorker | RepairOwner::DnsOwner => {
            if converges != 0 {
                return Verdict::Red(format!(
                    "non-owner shared-switch converge x{converges} while repairing {remaining:?}"
                ));
            }
            if audits != completed {
                return Verdict::Red(format!("full audit x{audits} for {completed} attempts"));
            }
        }
        RepairOwner::NoOwner => {
            if audits != completed {
                return Verdict::Red(format!("full audit x{audits} for {completed} attempts"));
            }
        }
    }
    let events = events_since(detected.events_at_arm);
    let retries = named(&events, "guest_network.shared_owner_retry");
    let recovered = named(&events, "guest_network.shared_owner_recovered");
    if named(&events, "guest_network.shared_owner_unhealthy").len() != 1
        || retries.len() != total_failed as usize
        || recovered.len() != 1
    {
        return Verdict::Red(format!(
            "operational events: unhealthy/retry/recovered = {}/{}/{} for {total_failed} failed attempts",
            named(&events, "guest_network.shared_owner_unhealthy").len(),
            retries.len(),
            recovered.len()
        ));
    }
    for (index, retry) in retries.iter().enumerate() {
        let attempt = index as u32 + 1;
        if retry.fields.get("attempt") != Some(&attempt.to_string()) {
            return Verdict::Red(format!("retry event {index} {retry:?}"));
        }
    }
    // Post-recovery preservation: admission stays open and no further quiesce.
    let calls_after_reopen = node.call_count();
    node.advance(Duration::from_secs(2)).await;
    let after = node.runtime_calls()[calls_after_reopen..].to_vec();
    if node.admission() != Admission::Open || node.progress().is_some() {
        return Verdict::Red(format!(
            "post-recovery admission {:?} progress {:?}",
            node.admission(),
            node.progress()
        ));
    }
    if count(&after, GuestNetworkOperation::TapSetDown) != 0 {
        return Verdict::Red(format!("post-recovery quiesce: {after:?}"));
    }
    Verdict::Green
}

// ---------------------------------------------------------------------------
// C5 — bounded window ends in one typed fail-stop request.
// ---------------------------------------------------------------------------

/// An unrepaired component fail-stops at exactly five seconds / twenty
/// completed attempts with one typed request, and admission refuses from then.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn unrepaired_loss_fail_stops_with_one_typed_request_at_the_deadline() {
    let mut report = Report::new("C5-deadline-typed-fail-stop");
    for seed in seeds() {
        for (index, component) in COMPONENTS.into_iter().enumerate() {
            let mut rng = cell_rng(seed, index);
            let phase = seeded_phase(&mut rng);
            let mut node = Node::boot(seed).await;
            let cell = format!("{component:?}");
            let verdict = match arm_and_detect(&mut node, component, phase).await {
                Err(missed) => Verdict::Unreached(format!("precondition detection: {missed}")),
                Ok(detected) => deadline_episode(&mut node, component, &detected).await,
            };
            report.record(seed, cell, verdict);
            node.shutdown().await;
        }
    }
    report.finish();
}

async fn deadline_episode(
    node: &mut Node,
    component: SharedGuestNetworkComponent,
    detected: &Detected,
) -> Verdict {
    let deadline = detected.at + RECOVERY_DEADLINE;
    while node.elapsed + STEP < deadline {
        node.step().await;
        if let Some((at, request)) = node.request.clone() {
            return Verdict::Red(format!(
                "request {request:?} before the deadline, {:?} after detection",
                at - detected.at
            ));
        }
        if node.admission() != Admission::Closed {
            return Verdict::Red(format!(
                "admission {:?} {:?} after detection",
                node.admission(),
                node.elapsed - detected.at
            ));
        }
    }
    node.advance(STEP * 3).await;
    let Some((at, request)) = node.request.clone() else {
        return Verdict::Red(format!(
            "no typed request {:?} after detection; progress {:?} admission {:?}",
            node.elapsed - detected.at,
            node.progress(),
            node.admission()
        ));
    };
    let expected = ServeShutdownRequest::SharedGuestNetwork(SharedGuestNetworkFailStop {
        component,
        cause: SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded,
        attempts: DEADLINE_ATTEMPTS,
        elapsed: RECOVERY_DEADLINE,
    });
    if request != expected || !within_one_step(at - detected.at, RECOVERY_DEADLINE) {
        return Verdict::Red(format!(
            "request {request:?} at {:?}; expected {expected:?} at {RECOVERY_DEADLINE:?}",
            at - detected.at
        ));
    }
    if node.admission() != Admission::FailStopped {
        return Verdict::Red(format!("admission {:?} after fail-stop", node.admission()));
    }
    Verdict::Green
}

// ---------------------------------------------------------------------------
// C6 — unconfirmed quiescence kills affected VMMs and fail-stops.
// ---------------------------------------------------------------------------

fn operator_client(config_dir: &Path) -> reqwest::Client {
    let contents = std::fs::read_to_string(config_dir.join(".overdrive/config"))
        .expect("operator trust config written at boot");
    let config: toml::Value = toml::from_str(&contents).expect("operator trust config parses");
    let encoded = config["contexts"]
        .as_array()
        .expect("contexts array")
        .iter()
        .find(|entry| entry["name"].as_str() == Some("local"))
        .expect("local context")["ca"]
        .as_str()
        .expect("ca field");
    let pem = base64::engine::general_purpose::STANDARD.decode(encoded).expect("ca base64");
    reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(&pem).expect("ca pem"))
        .https_only(true)
        .use_rustls_tls()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("operator client")
}

async fn submit_vm_job(client: &reqwest::Client, base: &str, id: &str) {
    let body = SubmitWorkloadRequest {
        spec: SubmitSpecInput::Job(JobSpecInput {
            id: id.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            driver: DriverInput::Vm(VmInput {
                command: "/bin/sleep".to_owned(),
                args: vec!["3600".to_owned()],
                kernel: "/kernel".to_owned(),
                rootfs: "/rootfs".to_owned(),
            }),
        }),
    };
    let response = client
        .post(format!("{base}/v1/workloads"))
        .json(&body)
        .send()
        .await
        .expect("submit reaches the public API");
    assert!(response.status().is_success(), "submit {id}: {response:?}");
    let _: SubmitWorkloadResponse = response.json().await.expect("submit response");
}

async fn running_allocations(obs: &SimObservationStore) -> BTreeSet<AllocationId> {
    obs.alloc_status_rows()
        .await
        .expect("sim observation rows")
        .into_iter()
        .filter(|row| row.state == AllocState::Running)
        .map(|row| row.alloc_id)
        .collect()
}

/// Deploy `count` VM jobs through the public HTTPS API and return the running
/// allocation ids once every job is Running.
async fn deploy_running_vms(node: &mut Node, count: usize) -> BTreeSet<AllocationId> {
    let bound = node
        .handle
        .as_ref()
        .expect("live handle")
        .local_addr()
        .await
        .expect("server bound address");
    let client = operator_client(&node.config_dir);
    let base = format!("https://localhost:{}", bound.port());
    for index in 0..count {
        submit_vm_job(&client, &base, &format!("nd295-proof-vm-{index}")).await;
    }
    for _ in 0..200 {
        node.clock.tick(Duration::from_millis(100));
        node.elapsed += Duration::from_millis(100);
        settle_io().await;
        let running = running_allocations(&node.obs).await;
        if running.len() >= count {
            return running;
        }
    }
    panic!("seed={:#x}: {count} VM jobs did not reach Running through the public API", node.seed);
}

/// When a kernel-path mismatch is detected and TAP quiescence cannot be
/// confirmed, every affected VM is killed before any repair is attempted and
/// the node takes the typed fail-stop path.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn unconfirmed_quiescence_kills_affected_vms_before_repair_and_fail_stops() {
    let mut report = Report::new("C6-unconfirmed-quiescence-kill-and-fail-stop");
    for seed in seeds() {
        for (index, component) in kernel_path_components().into_iter().enumerate() {
            let mut rng = cell_rng(seed, 100 + index);
            let phase = seeded_phase(&mut rng);
            let vm_count = rng.gen_range(1..=3_usize);
            let mut node = Node::boot(seed).await;
            let affected = deploy_running_vms(&mut node, vm_count).await;
            node.owner.sim.script_quiesce_failure(true);
            let cell = format!("{component:?}/vms={vm_count}");
            let verdict = match arm_and_detect(&mut node, component, phase).await {
                Err(missed) => Verdict::Unreached(format!("precondition detection: {missed}")),
                Ok(detected) => {
                    quiesce_failure_episode(&mut node, component, &detected, &affected).await
                }
            };
            report.record(seed, cell, verdict);
            node.shutdown().await;
        }
    }
    report.finish();
}

async fn quiesce_failure_episode(
    node: &mut Node,
    component: SharedGuestNetworkComponent,
    detected: &Detected,
    affected: &BTreeSet<AllocationId>,
) -> Verdict {
    let calls_at_detection = node.call_count();
    let limit = detected.at + RECOVERY_DEADLINE + STEP * 3;
    let mut first_repair_with_live_vm = None;
    let mut killed_at = None;
    while node.elapsed < limit && node.request.is_none() {
        node.clock.tick(STEP);
        node.elapsed += STEP;
        settle_io().await;
        let _ = node.poll_request();
        let running = running_allocations(&node.obs).await;
        let still_live: Vec<_> = affected.intersection(&running).cloned().collect();
        if still_live.is_empty() && killed_at.is_none() {
            killed_at = Some(node.elapsed);
        }
        let calls = node.runtime_calls()[calls_at_detection..].to_vec();
        if first_repair_with_live_vm.is_none()
            && !still_live.is_empty()
            && calls.contains(&GuestNetworkOperation::BridgeConverge)
        {
            first_repair_with_live_vm = Some((node.elapsed, still_live));
        }
    }
    let episode = node.runtime_calls()[detected.calls_at_arm..].to_vec();
    if !episode.contains(&GuestNetworkOperation::TapSetDown) {
        return Verdict::Red(format!("TAP quiescence never attempted: {episode:?}"));
    }
    if let Some((at, live)) = first_repair_with_live_vm {
        return Verdict::Red(format!(
            "repair converge at {:?} after detection while affected VMs {live:?} still ran",
            at - detected.at
        ));
    }
    let Some(killed_at) = killed_at else {
        return Verdict::Red(format!(
            "affected VMs {affected:?} were never killed within the bounded window"
        ));
    };
    let Some((requested_at, ServeShutdownRequest::SharedGuestNetwork(fail_stop))) =
        node.request.clone()
    else {
        return Verdict::Red(format!(
            "no typed fail-stop request within {RECOVERY_DEADLINE:?} after detection (kill at {:?})",
            killed_at - detected.at
        ));
    };
    if killed_at > requested_at {
        return Verdict::Red(format!(
            "fail-stop requested at {:?} before affected VMs were killed at {:?}",
            requested_at - detected.at,
            killed_at - detected.at
        ));
    }
    if fail_stop.component != component {
        return Verdict::Red(format!("fail-stop names {fail_stop:?}"));
    }
    if node.admission() != Admission::FailStopped {
        return Verdict::Red(format!("admission {:?} after fail-stop", node.admission()));
    }
    Verdict::Green
}

// ---------------------------------------------------------------------------
// C7 — late success cannot reopen after fail-stop.
// ---------------------------------------------------------------------------

/// After the typed fail-stop, an owner that becomes healthy again cannot
/// reopen admission, re-enter recovery, or receive further owner effects.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread")]
async fn healed_owner_after_fail_stop_cannot_reopen_admission() {
    let mut report = Report::new("C7a-post-fail-stop-heal-cannot-reopen");
    for seed in seeds() {
        for (index, component) in COMPONENTS.into_iter().enumerate() {
            let mut rng = cell_rng(seed, 200 + index);
            let phase = seeded_phase(&mut rng);
            let linger = Duration::from_secs(rng.gen_range(1..=10_u64));
            let mut node = Node::boot(seed).await;
            let cell = format!("{component:?}/linger={linger:?}");
            let verdict = match arm_and_detect(&mut node, component, phase).await {
                Err(missed) => Verdict::Unreached(format!("precondition detection: {missed}")),
                Ok(detected) => {
                    node.advance(RECOVERY_DEADLINE + STEP * 3).await;
                    if let Some((at, _)) = node.request.clone() {
                        let events_at_heal = events_cursor();
                        let calls_at_heal = node.call_count();
                        node.owner.sim.script_component_audit_failure(component, false);
                        let mut violation = None;
                        let until = node.elapsed + linger;
                        while node.elapsed < until {
                            node.step().await;
                            if violation.is_none()
                                && (node.admission() != Admission::FailStopped
                                    || node.progress().is_some())
                            {
                                violation = Some(format!(
                                    "admission {:?} progress {:?} {:?} after healing",
                                    node.admission(),
                                    node.progress(),
                                    node.elapsed - at
                                ));
                            }
                        }
                        let after = node.runtime_calls()[calls_at_heal..].to_vec();
                        let recovered = named(
                            &events_since(events_at_heal),
                            "guest_network.shared_owner_recovered",
                        )
                        .len();
                        let effects = after
                            .iter()
                            .filter(|call| {
                                matches!(
                                    call,
                                    GuestNetworkOperation::BridgeObserve
                                        | GuestNetworkOperation::BridgeConverge
                                        | GuestNetworkOperation::TapSetDown
                                )
                            })
                            .count();
                        violation
                            .or_else(|| {
                                (recovered != 0).then(|| {
                                    format!("{recovered} recovered events after fail-stop")
                                })
                            })
                            .or_else(|| {
                                (effects != 0)
                                    .then(|| format!("owner effects after fail-stop: {after:?}"))
                            })
                            .map_or(Verdict::Green, Verdict::Red)
                    } else {
                        Verdict::Unreached(format!(
                            "precondition fail-stop: no typed request {:?} after detection",
                            node.elapsed - detected.at
                        ))
                    }
                }
            };
            report.record(seed, cell, verdict);
            node.shutdown().await;
        }
    }
    report.finish();
}

/// An owner read-back still in flight at the five-second deadline cannot
/// extend the window, and its later success cannot reopen admission.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn in_flight_success_after_the_deadline_cannot_reopen_admission() {
    let mut report = Report::new("C7b-in-flight-late-success-cannot-reopen");
    for seed in seeds() {
        for (index, component) in COMPONENTS.into_iter().enumerate() {
            let mut rng = cell_rng(seed, 300 + index);
            let phase = seeded_phase(&mut rng);
            let hung_attempt = rng.gen_range(16..=DEADLINE_ATTEMPTS);
            let mut node = Node::boot(seed).await;
            let cell = format!("{component:?}/hung_attempt={hung_attempt}");
            let verdict = match arm_and_detect(&mut node, component, phase).await {
                Err(missed) => Verdict::Unreached(format!("precondition detection: {missed}")),
                Ok(detected) => {
                    in_flight_episode(&mut node, component, &detected, hung_attempt).await
                }
            };
            report.record(seed, cell, verdict);
            node.shutdown().await;
        }
    }
    report.finish();
}

async fn in_flight_episode(
    node: &mut Node,
    component: SharedGuestNetworkComponent,
    detected: &Detected,
    hung_attempt: u32,
) -> Verdict {
    for attempt in 1..hung_attempt {
        if await_attempt(node, attempt).await.is_none() {
            return Verdict::Unreached(format!(
                "precondition cadence: attempt {attempt} never completed"
            ));
        }
    }
    let latch = node.owner.arm_audit_latch();
    let deadline = detected.at + RECOVERY_DEADLINE;
    while node.elapsed < deadline + STEP * 3 {
        node.step().await;
    }
    let hung = node.owner.audits_in_flight();
    let request = node.request.clone();
    // Heal the owner and let the in-flight read-back finish successfully.
    node.owner.sim.script_component_audit_failure(component, false);
    node.owner.disarm_audit_latch();
    latch.add_permits(64);
    let events_at_release = events_cursor();
    node.advance(Duration::from_secs(2)).await;
    if hung == 0 {
        return Verdict::Unreached(format!(
            "precondition in-flight audit: attempt {hung_attempt} issued no owner audit"
        ));
    }
    let Some((at, ServeShutdownRequest::SharedGuestNetwork(fail_stop))) = request else {
        return Verdict::Red(format!(
            "a hung owner read-back extended the window: no typed request by {:?} after detection",
            deadline + STEP * 3 - detected.at
        ));
    };
    if !within_one_step(at - detected.at, RECOVERY_DEADLINE)
        || fail_stop.component != component
        || fail_stop.cause != SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded
        || fail_stop.attempts != hung_attempt - 1
        || fail_stop.elapsed != RECOVERY_DEADLINE
    {
        return Verdict::Red(format!(
            "fail-stop {fail_stop:?} at {:?}; expected {component:?}/RecoveryDeadlineExceeded/{} attempts at {RECOVERY_DEADLINE:?}",
            at - detected.at,
            hung_attempt - 1
        ));
    }
    let recovered =
        named(&events_since(events_at_release), "guest_network.shared_owner_recovered").len();
    if node.admission() != Admission::FailStopped || node.progress().is_some() || recovered != 0 {
        return Verdict::Red(format!(
            "late in-flight success reopened: admission {:?} progress {:?} recovered_events {recovered}",
            node.admission(),
            node.progress()
        ));
    }
    Verdict::Green
}

// ---------------------------------------------------------------------------
// C8 — supervisor task loss is observed immediately and fail-stops.
// ---------------------------------------------------------------------------

/// A retained-supervisor task loss (here: the panic class, induced by the
/// owner adapter panicking under the supervisor's own poll) is observed by
/// `ServerHandle` immediately, writes FailStop before the typed request
/// returns, and carries the snapshot the accepted contract fixes: no prior
/// recovery reports `Supervisor`/0/zero; in-progress recovery reports the
/// latest remaining component, completed attempts, and injected-clock elapsed.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn supervisor_task_loss_is_observed_immediately_and_fail_stops_with_the_latest_snapshot() {
    let mut report = Report::new("C8-supervisor-task-loss-fail-stop");
    for seed in seeds() {
        // (a) No prior recovery: the panic fires inside the periodic audit.
        {
            let mut rng = cell_rng(seed, 400);
            let phase = seeded_phase(&mut rng);
            let mut node = Node::boot(seed).await;
            node.advance(phase).await;
            node.owner.arm_audit_panic();
            let armed_at = node.elapsed;
            let mut panicked_at = None;
            while node.elapsed - armed_at < AUDIT_PERIOD + STEP && node.request.is_none() {
                node.step().await;
                if panicked_at.is_none() && node.owner.audit_panics() > 0 {
                    panicked_at = Some(node.elapsed);
                }
            }
            let cell = format!("panic-in-periodic-audit@phase={}ms", phase.as_millis());
            let verdict = match (panicked_at, node.request.clone()) {
                (None, _) => Verdict::Unreached(format!(
                    "precondition: the supervisor invoked no owner audit within one period of arming (owner calls since boot {:?})",
                    node.runtime_calls()
                )),
                (Some(at), None) => Verdict::Red(format!(
                    "supervisor task loss at {:?} after arming produced no typed request",
                    at - armed_at
                )),
                (Some(at), Some((requested_at, request))) => {
                    let expected =
                        ServeShutdownRequest::SharedGuestNetwork(SharedGuestNetworkFailStop {
                            component: SharedGuestNetworkComponent::Supervisor,
                            cause: SharedGuestNetworkFailStopCause::SupervisorPanicked,
                            attempts: 0,
                            elapsed: Duration::ZERO,
                        });
                    if request != expected {
                        Verdict::Red(format!("request {request:?}; expected {expected:?}"))
                    } else if requested_at > at {
                        Verdict::Red(format!(
                            "task loss at {at:?} observed late at {requested_at:?}"
                        ))
                    } else if node.admission() != Admission::FailStopped {
                        Verdict::Red(format!("admission {:?} after fail-stop", node.admission()))
                    } else {
                        Verdict::Green
                    }
                }
            };
            report.record(seed, cell, verdict);
            node.shutdown().await;
        }
        // (b) In-progress recovery for every snapshot component: the panic
        // fires inside the full audit of attempt `failed + 1`.
        for (index, component) in COMPONENTS.into_iter().enumerate() {
            let mut rng = cell_rng(seed, 500 + index);
            let phase = seeded_phase(&mut rng);
            let failed = rng.gen_range(1..=6_u32);
            let mut node = Node::boot(seed).await;
            let cell = format!("{component:?}/panic-after-{failed}-failed-attempts");
            let verdict = match arm_and_detect(&mut node, component, phase).await {
                Err(missed) => Verdict::Unreached(format!("precondition detection: {missed}")),
                Ok(detected) => {
                    let mut unreached = None;
                    for attempt in 1..=failed {
                        if await_attempt(&mut node, attempt).await.is_none() {
                            unreached = Some(format!(
                                "precondition cadence: attempt {attempt} never completed"
                            ));
                            break;
                        }
                    }
                    if let Some(reason) = unreached {
                        Verdict::Unreached(reason)
                    } else {
                        node.owner.arm_audit_panic();
                        let armed_at = node.elapsed;
                        let mut panicked_at = None;
                        while node.elapsed - armed_at < ATTEMPT_PERIOD + STEP
                            && node.request.is_none()
                        {
                            node.step().await;
                            if panicked_at.is_none() && node.owner.audit_panics() > 0 {
                                panicked_at = Some(node.elapsed);
                            }
                        }
                        match (panicked_at, node.request.clone()) {
                            (None, _) => Verdict::Unreached(format!(
                                "precondition: attempt {} issued no full owner audit",
                                failed + 1
                            )),
                            (Some(at), None) => Verdict::Red(format!(
                                "supervisor task loss {:?} after detection produced no typed request",
                                at - detected.at
                            )),
                            (
                                Some(at),
                                Some((
                                    requested_at,
                                    ServeShutdownRequest::SharedGuestNetwork(fail_stop),
                                )),
                            ) => {
                                let expected_elapsed = ATTEMPT_PERIOD * (failed + 1);
                                if fail_stop.component != component
                                    || fail_stop.cause
                                        != SharedGuestNetworkFailStopCause::SupervisorPanicked
                                    || fail_stop.attempts != failed
                                    || !within_one_step(fail_stop.elapsed, expected_elapsed)
                                {
                                    Verdict::Red(format!(
                                        "request {fail_stop:?}; expected {component:?}/SupervisorPanicked/{failed} attempts/{expected_elapsed:?}"
                                    ))
                                } else if requested_at > at {
                                    Verdict::Red(format!(
                                        "task loss at {:?} observed late at {:?}",
                                        at - detected.at,
                                        requested_at - detected.at
                                    ))
                                } else if node.admission() != Admission::FailStopped {
                                    Verdict::Red(format!(
                                        "admission {:?} after fail-stop",
                                        node.admission()
                                    ))
                                } else {
                                    Verdict::Green
                                }
                            }
                        }
                    }
                }
            };
            report.record(seed, cell, verdict);
            node.shutdown().await;
        }
    }
    report.finish();
}
