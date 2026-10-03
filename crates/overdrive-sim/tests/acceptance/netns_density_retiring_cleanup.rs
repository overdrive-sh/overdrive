//! S-ND295-07 (E8 seeded) — a stopping allocation whose protection removal
//! fails keeps its guest address until a retry completes, whatever the
//! interleaving of retries and other starts (D-295-R7, D-295-R10; FD §
//! "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8)
//! — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the retirement
//! points and the lease events); FD § "[REF] Driven port — intercept element
//! release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) —
//! ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)"
//! (`remove_allocation_elements`)).
//!
//! # Contract under test
//!
//! E8, retry-retaining cleanup: a failing element removal keeps the stopping
//! allocation's lease held (Retiring) and counted, admits no successor on its
//! address, and a retry converges. The lease is released only after the
//! allocation's cleanup has finished, so while it is held no start receives
//! that address; once `guest_network.lease_released { alloc }` is seen, the
//! pool's smallest-free selection hands the address to the next start.
//!
//! # Production owner path
//!
//! Every decision runs through
//! `run_convergence_tick_with_guest_network_provisioner_for_test`: the
//! registered `WorkloadLifecycle` places each Service (`StartAllocation`) and,
//! under an operator stop, emits `StopAllocation` for every `Running` row on
//! every evaluation, so a stop whose cleanup failed (its row stays `Running`)
//! is retried by the next evaluation. The production action shim takes each
//! lease from the server's pool, retires it before the stop's first cleanup
//! effect, removes the allocation's shared protection members through the
//! worker, tears the network down, and releases the lease last. Operator
//! input reaches the real `LocalIntentStore` with the store effects of
//! `handlers::submit_workload` and `handlers::stop_workload`.
//!
//! # Seam fixture and fault
//!
//! One owner instance, `SimSharedGuestNetworkOwner` behind a recording
//! decorator that journals each `provision` with its assigned address; one
//! `GuestNetworkExecWiring` over the fixture `SimClock`, opened before the
//! first dispatch; one `GuestAddressPool` with today's constants; one started
//! `MtlsInterceptWorker` over `SimMtlsEnforcement`, `SimMtlsResolve`, and a
//! test-local `MtlsIntercept` that wraps `SimMtlsIntercept`. The fault enters
//! only through that intercept: while armed for the predecessor's guest
//! address, `remove_allocation_elements` for that source is rejected before
//! any effect (the batch-rejected shape of the port contract). Every other
//! call, and every other allocation's removal, delegates unchanged. The
//! decorators observe and inject; they author no lease, row, or decision.
//! Until DELIVER 05-01 the `AppState` constructor takes today's inputs and the
//! fixture holds the worker, gate, and pool; 05-01 changes that one call.
//!
//! # Schedule
//!
//! The predecessor is placed first, so its address is the lowest the pool has
//! handed out. Each seed chooses how many other Services start before the
//! stop, how many stop attempts fail (the first stop always does), how many
//! other Services start and how many of them stop while the predecessor's
//! lease is held, and the order of those events and the simulated time
//! between them. Then the fault is disarmed, the stop is retried until the
//! lease is released, and one more Service starts.
//!
//! # Oracle
//!
//! One ordered journal holds every `provision` (allocation and assigned
//! address), every `remove_allocation_elements` outcome, the pinned lease
//! events `guest_network.lease_retired { alloc }` and
//! `guest_network.lease_released { alloc }` captured by a test-local tracing
//! `Layer`, and the arm/disarm marks:
//!
//! - every failing stop attempt reached the removal for the predecessor's
//!   source, and the lease was retired before the first refused removal;
//! - no `lease_released` for the predecessor precedes the disarm;
//! - every `provision` before the predecessor's `lease_released` is for a
//!   different address;
//! - after the disarm exactly one `lease_released` for the predecessor
//!   follows a removal of its source that succeeded;
//! - the next start's `provision`, and the network assignment in the spec the
//!   `SimDriver` received for it (the C-295-A handoff), carry the
//!   predecessor's address.
//!
//! # Evidence tier and reproduction
//!
//! Tier 1, default lane: current-thread runtime, no kernel object, no wall
//! time. The only clock is the fixture's `SimClock`, advanced by the harness
//! between events; every wait is an awaited evaluation. Seeds come from
//! `OVERDRIVE_ND295_RETIRING_CLEANUP_SEEDS` (a comma-separated `u64` list,
//! default [`DEFAULT_SEEDS`]) and are printed with every verdict. Reproduce
//! with `OVERDRIVE_ND295_RETIRING_CLEANUP_SEEDS=<seed> cargo xtask lima run --
//! cargo nextest run -p overdrive-sim --test acceptance --run-ignored
//! ignored-only --no-capture -E 'test(/netns_density_retiring_cleanup/)'`.

#![allow(
    clippy::print_stderr,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::large_futures,
    reason = "seeded acceptance harness: every verdict prints its seed; fixture preconditions fail loudly"
)]

use std::collections::{BTreeMap, VecDeque};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use overdrive_control_plane::guest_network::{
    GuestAddressPool, GuestNetworkPlan, GuestNetworkProvisioner, Result as GuestNetworkResult,
    SharedGuestNetworkAudit, SharedGuestNetworkAuditError, SharedGuestNetworkOwner, TapActivation,
    TapQuiescence,
};
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, run_convergence_tick_with_guest_network_provisioner_for_test,
};
use overdrive_control_plane::{AppState, workload_lifecycle};
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, Service, VmInput, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::{ListenerInput, ServiceSpecInput};
use overdrive_core::guest_network::{
    GuestNetworkExecGate, GuestNetworkExecSupervisor, GuestNetworkExecWiring,
};
use overdrive_core::id::{AllocationId, MeshServiceName, NodeId, WorkloadId};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::{IntentStore, PutOutcome};
use overdrive_core::traits::mtls_enforcement::MtlsLimits;
use overdrive_core::traits::mtls_resolve::MtlsResolution;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::view_store::SimViewStore;
use overdrive_sim::adapters::{
    SimIdentityRead, SimMtlsEnforcement, SimMtlsIntercept, SimMtlsResolve,
};
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::mtls_intercept::{
    InterceptElementKey, InterceptElementOperation, InterceptError, InterceptPostcondition,
    InterceptSet, NetlinkError, Result as InterceptResult,
};
use overdrive_worker::mtls_intercept_port::{
    InterceptGuard, InterceptListener, InterceptMembers, InterceptState, MtlsIntercept,
};
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;
use parking_lot::Mutex;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use tempfile::TempDir;
use tracing::subscriber::DefaultGuard;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

const SEEDS_ENV: &str = "OVERDRIVE_ND295_RETIRING_CLEANUP_SEEDS";
const DEFAULT_SEEDS: [u64; 4] =
    [0x0295_0007_0000_0001, 0x0295_0007_0000_0002, 0x0295_0007_0000_0003, 0x0295_0007_0000_0004];
/// The Phase-1 single-node baseline id (`workload_lifecycle::baseline_nodes_phase1`).
const NODE: &str = "local";
/// Today's pool constants (`guest_network::action_pool`): node guest prefix,
/// bridge, and the gateway that is also the guest DNS address.
const GUEST_PREFIX: &str = "100.95.0.0/16";
const GUEST_BRIDGE: &str = "ovd-gbr0";
const GUEST_GATEWAY: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
/// The listener every Service declares; it is the inbound destination the
/// stop removes.
const SERVICE_PORT: u16 = 8080;
/// Bounded evaluations for a stop whose removal can succeed to release its
/// lease.
const SETTLE_EVALUATIONS: usize = 3;
/// Simulated time between schedule events, in milliseconds.
const GAP_MIN_MS: u64 = 50;
const GAP_MAX_MS: u64 = 500;
const LEASE_RETIRED: &str = "guest_network.lease_retired";
const LEASE_RELEASED: &str = "guest_network.lease_released";
/// The `op` of the element-removal failure the fault intercept injects.
const INJECTED_REMOVAL_OP: &str = "nd295-e8-injected-member-delete";

fn seeds() -> Vec<u64> {
    std::env::var(SEEDS_ENV).ok().map_or_else(
        || DEFAULT_SEEDS.to_vec(),
        |raw| {
            raw.split(',')
                .map(|seed| {
                    seed.trim().parse().unwrap_or_else(|_| {
                        panic!("{SEEDS_ENV} must be a comma-separated list of u64 seeds: {raw}")
                    })
                })
                .collect()
        },
    )
}

/// Setup preconditions fail loudly with the seed: a harness failure, never a
/// contract verdict.
trait Setup<T> {
    fn setup(self, seed: u64, what: &str) -> T;
}

impl<T, E: std::fmt::Debug> Setup<T> for Result<T, E> {
    fn setup(self, seed: u64, what: &str) -> T {
        self.unwrap_or_else(|error| {
            panic!("seed={seed}: harness precondition failed (not a contract verdict): {what}: {error:?}")
        })
    }
}

// ---------------------------------------------------------------------------
// Ordered observation journal (driven ports + lease events + test marks).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    FaultArmed,
    FaultDisarmed,
}

#[derive(Debug, Clone)]
enum Entry {
    /// `provision` reached the owner port: the shim took this lease.
    Provision {
        alloc: AllocationId,
        address: Ipv4Addr,
    },
    /// `remove_allocation_elements` returned at the intercept port.
    Removal {
        source: Ipv4Addr,
        refused: bool,
    },
    /// A pinned lease event; `alloc` is the rendered `alloc` field, `None`
    /// when the event carried none.
    Lease {
        name: &'static str,
        alloc: Option<String>,
    },
    Mark(Mark),
}

impl std::fmt::Display for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provision { alloc, address } => write!(f, "provision {alloc} at {address}"),
            Self::Removal { source, refused } => {
                write!(f, "remove_allocation_elements({source}) refused={refused}")
            }
            Self::Lease { name, alloc } => write!(f, "{name} {alloc:?}"),
            Self::Mark(mark) => write!(f, "mark {mark:?}"),
        }
    }
}

#[derive(Clone, Default)]
struct Journal(Arc<Mutex<Vec<Entry>>>);

impl Journal {
    fn push(&self, entry: Entry) {
        self.0.lock().push(entry);
    }

    fn len(&self) -> usize {
        self.0.lock().len()
    }

    fn entries(&self) -> Vec<Entry> {
        self.0.lock().clone()
    }

    fn render(&self) -> String {
        self.0
            .lock()
            .iter()
            .enumerate()
            .map(|(index, entry)| format!("  #{index} {entry}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Whether `entry` is the lease event `name` naming exactly `alloc`.
fn is_lease_event(entry: &Entry, name: &str, alloc: &AllocationId) -> bool {
    matches!(entry, Entry::Lease { name: found, alloc: Some(named) }
        if *found == name && named == alloc.as_str())
}

/// The allocation an `alloc` field names, whether it was recorded with `%`
/// (`alloc-x-0`) or `?` (`AllocationId("alloc-x-0")`, or a quoted string).
fn rendered_alloc(raw: &str) -> String {
    let raw = raw.trim_matches('"');
    raw.strip_prefix("AllocationId(\"")
        .and_then(|inner| inner.strip_suffix("\")"))
        .unwrap_or(raw)
        .to_owned()
}

struct FieldText<'a>(&'a mut BTreeMap<String, String>);

impl tracing::field::Visit for FieldText<'_> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
}

/// Test-local `Layer` journaling the pinned `lease_retired` and
/// `lease_released` events. Thread-local capture suffices: the runtime is
/// current-thread and every evaluation dispatches on it.
struct LeaseCapture(Journal);

impl<S> Layer<S> for LeaseCapture
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let name = match event.metadata().name() {
            LEASE_RETIRED => LEASE_RETIRED,
            LEASE_RELEASED => LEASE_RELEASED,
            _ => return,
        };
        let mut fields = BTreeMap::new();
        event.record(&mut FieldText(&mut fields));
        self.0
            .push(Entry::Lease { name, alloc: fields.get("alloc").map(|raw| rendered_alloc(raw)) });
    }
}

// ---------------------------------------------------------------------------
// Driven-port decorators (observation and fault injection only).
// ---------------------------------------------------------------------------

/// The one owner instance: `SimSharedGuestNetworkOwner` behind a decorator
/// that journals every `provision` with the address the shim assigned. It
/// implements both owner ports by delegation and authors nothing.
struct JournalOwner {
    inner: SimSharedGuestNetworkOwner,
    journal: Journal,
}

#[async_trait::async_trait]
impl GuestNetworkProvisioner for JournalOwner {
    async fn provision(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        self.journal.push(Entry::Provision {
            alloc: plan.alloc().clone(),
            address: plan.assignment().address,
        });
        self.inner.provision(plan).await
    }

    async fn activate(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<TapActivation> {
        self.inner.activate(plan).await
    }

    async fn teardown(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        self.inner.teardown(plan).await
    }
}

#[async_trait::async_trait]
impl SharedGuestNetworkOwner for JournalOwner {
    async fn probe_startup(&self) -> GuestNetworkResult<()> {
        self.inner.probe_startup().await
    }

    async fn sweep_stale(&self) -> GuestNetworkResult<()> {
        self.inner.sweep_stale().await
    }

    async fn converge_shared(&self) -> GuestNetworkResult<()> {
        self.inner.converge_shared().await
    }

    async fn audit_shared(&self) -> Result<SharedGuestNetworkAudit, SharedGuestNetworkAuditError> {
        self.inner.audit_shared().await
    }

    async fn quiesce_managed_taps(&self) -> GuestNetworkResult<TapQuiescence> {
        self.inner.quiesce_managed_taps().await
    }

    async fn restore_quiesced_taps(&self) -> GuestNetworkResult<()> {
        self.inner.restore_quiesced_taps().await
    }
}

/// The listener type `MtlsIntercept::bind_transparent` returns. The DELIVER
/// step that carries B-7 (05-01 at the latest) changes this one line to
/// `Arc<dyn InterceptListener>` (FD § "[REF] Driven port — intercept listener
/// (DISTILL gap B-7) — pinned 2026-09-25" (the pinned `bind_transparent`
/// signature)); the delegation below is unchanged by that step.
type BoundListener = Arc<dyn InterceptListener>;

/// The removal failure a rejected delete batch reports (the
/// `remove_allocation_elements` contract): the managed-guest member of
/// `source` could not be deleted, `EBUSY`, and nothing changed.
fn injected_removal_failure(source: Ipv4Addr) -> InterceptError {
    InterceptError::NftElementUpdateFailed {
        set: InterceptSet::ManagedGuestIps,
        operation: InterceptElementOperation::Delete,
        key: InterceptElementKey::Address(source),
        source: NetlinkError::nft(
            INJECTED_REMOVAL_OP,
            std::io::Error::from_raw_os_error(libc::EBUSY),
        ),
    }
}

/// `SimMtlsIntercept` whose element removals enter the journal, with one
/// standing removal fault keyed on a source address: while armed for
/// `source`, a removal for that source is rejected before any effect.
struct RemovalFaultIntercept {
    inner: SimMtlsIntercept,
    journal: Journal,
    fault: Mutex<Option<Ipv4Addr>>,
}

impl RemovalFaultIntercept {
    fn new(journal: Journal) -> Self {
        Self { inner: SimMtlsIntercept::new(), journal, fault: Mutex::new(None) }
    }

    fn arm(&self, source: Ipv4Addr) {
        *self.fault.lock() = Some(source);
        self.journal.push(Entry::Mark(Mark::FaultArmed));
    }

    fn disarm(&self) {
        *self.fault.lock() = None;
        self.journal.push(Entry::Mark(Mark::FaultDisarmed));
    }
}

impl MtlsIntercept for RemovalFaultIntercept {
    fn bind_transparent(&self, addr: SocketAddrV4) -> InterceptResult<BoundListener> {
        self.inner.bind_transparent(addr)
    }

    fn converge_shared(
        &self,
        prior: Option<&InterceptPostcondition>,
        leg_f: SocketAddrV4,
        leg_c: SocketAddrV4,
    ) -> InterceptResult<Box<dyn InterceptGuard>> {
        self.inner.converge_shared(prior, leg_f, leg_c)
    }

    fn observe_shared(&self) -> InterceptResult<Option<InterceptPostcondition>> {
        self.inner.observe_shared()
    }

    fn install_outbound(
        &self,
        source_addr: Ipv4Addr,
        agent_leg_f_port: u16,
    ) -> InterceptResult<Box<dyn InterceptGuard>> {
        self.inner.install_outbound(source_addr, agent_leg_f_port)
    }

    fn install_inbound(
        &self,
        virt: SocketAddrV4,
        agent_leg_c_port: u16,
    ) -> InterceptResult<Box<dyn InterceptGuard>> {
        self.inner.install_inbound(virt, agent_leg_c_port)
    }

    fn observe_shared_state(&self) -> InterceptResult<Option<InterceptState>> {
        self.inner.observe_shared_state()
    }

    fn converge_allocation_elements(
        &self,
        expected: &InterceptMembers,
    ) -> InterceptResult<Option<InterceptState>> {
        self.inner.converge_allocation_elements(expected)
    }

    fn remove_allocation_elements(
        &self,
        source_addr: Ipv4Addr,
        destinations: &[SocketAddrV4],
    ) -> InterceptResult<InterceptState> {
        let armed = *self.fault.lock() == Some(source_addr);
        let result = if armed {
            Err(injected_removal_failure(source_addr))
        } else {
            self.inner.remove_allocation_elements(source_addr, destinations)
        };
        self.journal.push(Entry::Removal { source: source_addr, refused: result.is_err() });
        result
    }
}

// ---------------------------------------------------------------------------
// Seam fixture (TS § Seam fixture).
// ---------------------------------------------------------------------------

struct Fixture {
    _tmp: TempDir,
    seed: u64,
    state: AppState,
    clock: Arc<SimClock>,
    driver: Arc<SimDriver>,
    owner: Arc<JournalOwner>,
    intercept: Arc<RemovalFaultIntercept>,
    supervisor: Arc<GuestNetworkExecSupervisor>,
    #[allow(dead_code, reason = "passed to AppState by DELIVER 05-01's pinned constructor")]
    gate: Arc<GuestNetworkExecGate>,
    #[allow(dead_code, reason = "passed to AppState by DELIVER 05-01's pinned constructor")]
    pool: Arc<GuestAddressPool>,
    #[allow(dead_code, reason = "passed to AppState by DELIVER 05-01's pinned constructor")]
    worker: Arc<MtlsInterceptWorker>,
    journal: Journal,
    reconciler: ReconcilerName,
    tick: AtomicU64,
    notes: Mutex<Vec<String>>,
}

impl Fixture {
    /// This file's seam fixture helper (TS § *Seam fixture*).
    async fn compose(seed: u64) -> Self {
        let tmp = TempDir::new().setup(seed, "tempdir");
        let mut runtime = ReconcilerRuntime::new(tmp.path(), Arc::new(SimViewStore::new()))
            .setup(seed, "reconciler runtime");
        runtime.register(workload_lifecycle()).await.setup(seed, "register workload-lifecycle");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).setup(seed, "intent store"));
        let node_id = NodeId::new(NODE).setup(seed, "node id");
        let obs: Arc<dyn ObservationStore> =
            Arc::new(SimObservationStore::single_peer(node_id.clone(), seed));
        let clock = Arc::new(SimClock::new());
        let driver = Arc::new(SimDriver::with_clock(DriverType::Vm, clock.clone()));
        let journal = Journal::default();

        // The one owner instance: the seam's `provisioner` and AppState's owner.
        let owner = Arc::new(JournalOwner {
            inner: SimSharedGuestNetworkOwner::default(),
            journal: journal.clone(),
        });
        // One EXEC wiring over the fixture clock; the gate starts BootClosed
        // and only the kept supervisor moves it.
        let wiring = GuestNetworkExecWiring::new(clock.clone() as Arc<dyn Clock>);
        let gate = wiring.gate();
        let supervisor = wiring.supervisor();
        // One pool from the doc-hidden constructor with today's constants.
        let pool = Arc::new(GuestAddressPool::new(
            GUEST_PREFIX.parse().setup(seed, "static guest prefix"),
            GUEST_BRIDGE.to_owned(),
            GUEST_GATEWAY,
            GUEST_GATEWAY,
        ));
        // One worker over the sim mTLS ports and the fault intercept, its
        // shared owner started (it binds nothing over the sim port from B-7).
        let intercept = Arc::new(RemovalFaultIntercept::new(journal.clone()));
        let worker = Arc::new(MtlsInterceptWorker::new(
            Arc::new(SimMtlsEnforcement::new(
                Arc::new(SimIdentityRead::new(BTreeMap::new(), None)),
                MtlsLimits::default(),
            )),
            Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh)),
            clock.clone() as Arc<dyn Clock>,
            Arc::clone(&intercept) as Arc<dyn MtlsIntercept>,
        ));
        worker.start_shared_owner().await.unwrap_or_else(|error| {
            panic!(
                "seed={seed}: harness precondition failed (not a contract verdict): the worker's \
                 shared owner did not start over the sim ports: {error}"
            )
        });

        let allocator =
            overdrive_control_plane::test_default_allocator(store.clone() as Arc<dyn IntentStore>);
        // DELIVER 05-01 changes this one call: the pinned `AppState::new`
        // appends `worker`, `owner`, `gate`, and `pool` as required parameters
        // (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED
        // 2026-09-24" (the `AppState` constructors)), passed here as
        // `Arc::clone(&worker)`,
        // `Arc::clone(&owner) as Arc<dyn SharedGuestNetworkOwner>`,
        // `Arc::clone(&gate)`, and `Arc::clone(&pool)`. Until then the fixture
        // keeps them for its body.
        let state = AppState::new(
            store,
            store_path,
            obs,
            Arc::new(runtime),
            driver.clone() as Arc<dyn Driver>,
            clock.clone() as Arc<dyn Clock>,
            Arc::new(SimDataplane::new()),
            Arc::new(SimCa::new(Arc::new(SimEntropy::new(seed)))),
            Arc::new(IdentityMgr::new(None)),
            node_id,
            allocator,
            overdrive_control_plane::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            Arc::clone(&worker),
            Arc::clone(&owner) as Arc<dyn SharedGuestNetworkOwner>,
            Arc::clone(&gate),
            Arc::clone(&pool),
        );
        Self {
            _tmp: tmp,
            seed,
            state,
            clock,
            driver,
            owner,
            intercept,
            supervisor,
            gate,
            pool,
            worker,
            journal,
            reconciler: ReconcilerName::new("workload-lifecycle").setup(seed, "reconciler name"),
            tick: AtomicU64::new(0),
            notes: Mutex::new(Vec::new()),
        }
    }

    fn capture(&self) -> DefaultGuard {
        tracing::subscriber::set_default(
            Registry::default().with(LeaseCapture(self.journal.clone())),
        )
    }

    fn note(&self, line: String) {
        self.notes.lock().push(line);
    }

    fn evidence(&self) -> String {
        format!(
            "trace:\n  {}\njournal:\n{}\nreproduce: {SEEDS_ENV}={} cargo xtask lima run -- cargo \
             nextest run -p overdrive-sim --test acceptance --run-ignored ignored-only \
             --no-capture -E 'test(/netns_density_retiring_cleanup/)'",
            self.notes.lock().join("\n  "),
            self.journal.render(),
            self.seed,
        )
    }

    fn harness_failure(&self, what: &str) -> ! {
        panic!(
            "seed={}: harness precondition failed (not a contract verdict): {what}\n{}",
            self.seed,
            self.evidence()
        );
    }

    /// A contract verdict: `holds`, or fail with the seed and the evidence.
    fn verdict(&self, id: &str, holds: bool, what: &str) {
        assert!(holds, "seed={}: [RED] {id}: {what}\n{}", self.seed, self.evidence());
        eprintln!("seed={} [GREEN] {id}", self.seed);
    }

    /// The gate every activation claims: opened once, as `run_server*`'s boot
    /// sequence does (FD § "[REF] Driven port — TAP activation gate
    /// (D-295-R5) — ACCEPTED 2026-09-24" (gate state outside `run_server*`)).
    fn open_gate(&self) {
        if !self.supervisor.open_after_boot() {
            self.harness_failure("open_after_boot was refused");
        }
    }

    /// Advance simulated time by a seeded gap between events.
    fn gap(&self, rng: &mut StdRng) {
        self.clock.tick(Duration::from_millis(rng.gen_range(GAP_MIN_MS..=GAP_MAX_MS)));
    }

    /// Store and allocator effects of `handlers::submit_workload` for a
    /// Service intent.
    async fn operator_deploys_service(&self, workload: &str) {
        let service = Service::from_submit(ServiceSpecInput {
            id: workload.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 0, memory_bytes: 256 * 1024 },
            driver: DriverInput::Vm(VmInput {
                command: "/bin/sleep".to_owned(),
                args: vec!["3600".to_owned()],
                kernel: "/nd295/kernel".to_owned(),
                rootfs: "/nd295/rootfs.ext4".to_owned(),
            }),
            listeners: vec![ListenerInput { port: SERVICE_PORT, protocol: "tcp".to_owned() }],
            startup_probes: Vec::new(),
            readiness_probes: Vec::new(),
            liveness_probes: Vec::new(),
        })
        .setup(self.seed, "valid service spec");
        let intent = WorkloadIntent::Service(service.clone());
        let archived = intent.archive_for_store().setup(self.seed, "archive workload intent");
        let digest = intent.spec_digest().setup(self.seed, "spec digest");
        if let Ok(name) =
            MeshServiceName::new(&format!("{}.{}", service.id.as_str(), MeshServiceName::SUFFIX))
        {
            self.state.frontend_addr_allocator.assign(&name).setup(self.seed, "frontend address");
        }
        let vip = {
            let mut guard = self.state.allocator.lock().await;
            guard.allocate(*digest.as_bytes()).await.setup(self.seed, "service vip")
        };
        let key = IntentKey::for_workload(&service.id);
        let outcome = self
            .state
            .store
            .put_if_absent(key.as_bytes(), archived.as_ref())
            .await
            .setup(self.seed, "intent write");
        if !matches!(outcome, PutOutcome::Inserted) {
            self.harness_failure(&format!("service {workload} was not fresh"));
        }
        let kind_key = IntentKey::for_workload_kind(&service.id);
        self.state
            .store
            .put(kind_key.as_bytes(), &[WorkloadKind::Service.discriminator_byte()])
            .await
            .setup(self.seed, "kind write");
        self.state.listener_facts.lock().await.upsert(service.id.clone(), &vip, &service.listeners);
        self.note(format!("operator deploys service {workload}"));
    }

    /// Store effect of `handlers::stop_workload`.
    async fn operator_stops(&self, workload: &str) {
        let id = WorkloadId::new(workload).setup(self.seed, "workload id");
        let stop_key = IntentKey::for_workload_stop(&id);
        self.state
            .store
            .put_if_absent(stop_key.as_bytes(), b"")
            .await
            .setup(self.seed, "stop write");
        self.note(format!("operator stops {workload}"));
    }

    /// One production `workload-lifecycle` evaluation for `workload`. Its
    /// outcome is recorded in the trace; the oracle reads the journal.
    async fn converge(&self, workload: &str) {
        let n = self.tick.fetch_add(1, Ordering::SeqCst);
        let now = self.clock.now();
        let target =
            TargetResource::new(&format!("workload/{workload}")).setup(self.seed, "target");
        let outcome = Box::pin(run_convergence_tick_with_guest_network_provisioner_for_test(
            &self.state,
            &self.reconciler,
            &target,
            now,
            n,
            now + Duration::from_secs(1),
            self.owner.as_ref(),
        ))
        .await;
        let line = match outcome {
            Ok(()) => format!("evaluation {n} of {workload}: Ok"),
            Err(error) => format!("evaluation {n} of {workload}: {error:?}"),
        };
        self.note(line);
    }

    /// The `provision` entries at or after journal index `from` whose
    /// allocation belongs to `workload` (`alloc-<workload>-<attempt>`).
    fn provisions_of(&self, workload: &str, from: usize) -> Vec<(AllocationId, Ipv4Addr)> {
        let prefix = format!("alloc-{workload}-");
        self.journal
            .entries()
            .into_iter()
            .skip(from)
            .filter_map(|entry| match entry {
                Entry::Provision { alloc, address } if alloc.as_str().starts_with(&prefix) => {
                    Some((alloc, address))
                }
                _ => None,
            })
            .collect()
    }

    /// Deploy and place a Service through the production evaluation; return
    /// the allocation and the address its lease carries.
    async fn place_service(&self, workload: &str) -> (AllocationId, Ipv4Addr) {
        let from = self.journal.len();
        self.operator_deploys_service(workload).await;
        self.converge(workload).await;
        let provisions = self.provisions_of(workload, from);
        let [(alloc, address)] = provisions.as_slice() else {
            self.harness_failure(&format!(
                "the placement of {workload} did not acquire exactly one attachment: {provisions:?}"
            ));
        };
        self.note(format!("{workload} placed as {alloc} at {address}"));
        (alloc.clone(), *address)
    }

    /// Whether `alloc`'s `lease_released` has been journaled.
    fn released(&self, alloc: &AllocationId) -> bool {
        self.journal.entries().iter().any(|entry| is_lease_event(entry, LEASE_RELEASED, alloc))
    }

    /// Stop `workload` and converge until `alloc`'s lease is released; its
    /// removal is not faulted, so a missing release is a harness failure.
    async fn stop_and_settle(&self, workload: &str, alloc: &AllocationId) {
        self.operator_stops(workload).await;
        for _ in 0..SETTLE_EVALUATIONS {
            self.converge(workload).await;
            if self.released(alloc) {
                return;
            }
        }
        self.harness_failure(&format!(
            "{workload}'s unfaulted stop did not release {alloc}'s lease within \
             {SETTLE_EVALUATIONS} evaluations"
        ));
    }

    /// Refused removals of `source` journaled at or after index `from`.
    fn refused_removals(&self, source: Ipv4Addr, from: usize) -> usize {
        self.journal
            .entries()
            .iter()
            .skip(from)
            .filter(|entry| {
                matches!(entry, Entry::Removal { source: found, refused: true } if *found == source)
            })
            .count()
    }
}

fn tag(seed: u64) -> String {
    format!("{seed:016x}")
}

/// One event while the predecessor's lease is held.
#[derive(Debug, Clone, Copy)]
enum HoldEvent {
    /// Another evaluation of the stopping workload with the fault armed: the
    /// stop is retried and its removal refused again.
    FailingRetry,
    /// A fresh Service is deployed and placed: the start that would take the
    /// predecessor's address if its lease were free.
    SuccessorStart,
    /// The earliest still-running other Service is stopped and its lease
    /// released, freeing an address other than the predecessor's.
    SuccessorStop,
}

async fn retiring_cleanup_schedule(seed: u64) {
    let mut rng = StdRng::seed_from_u64(seed);
    let pre_stop_starts = rng.gen_range(0..=1_usize);
    let failing_attempts = rng.gen_range(1..=3_usize);
    let hold_starts = rng.gen_range(1..=3_usize);
    let hold_stops = rng.gen_range(0..=1_usize);
    let mut events: Vec<HoldEvent> =
        std::iter::repeat_n(HoldEvent::FailingRetry, failing_attempts - 1)
            .chain(std::iter::repeat_n(HoldEvent::SuccessorStart, hold_starts))
            .chain(std::iter::repeat_n(HoldEvent::SuccessorStop, hold_stops))
            .collect();
    events.shuffle(&mut rng);
    eprintln!(
        "seed={seed} S-ND295-07 E8 schedule: starts before the stop={pre_stop_starts} failing stop \
         attempts={failing_attempts} hold events={events:?}"
    );

    let fixture = Fixture::compose(seed).await;
    let _capture = fixture.capture();
    fixture.open_gate();
    fixture.note(format!(
        "schedule: starts before the stop={pre_stop_starts} failing stop attempts=\
         {failing_attempts} hold events={events:?}"
    ));
    let tag = tag(seed);

    // The predecessor is placed first, so its address is the lowest handed out.
    let predecessor = format!("e8p-{tag}");
    let (pred_alloc, pred_address) = fixture.place_service(&predecessor).await;
    let mut running: VecDeque<(String, AllocationId)> = VecDeque::new();
    let mut successor_index = 0_usize;
    for _ in 0..pre_stop_starts {
        fixture.gap(&mut rng);
        let workload = format!("e8s-{tag}-{successor_index}");
        successor_index += 1;
        let (alloc, _) = fixture.place_service(&workload).await;
        running.push_back((workload, alloc));
    }

    // The first stop: its removal is refused, so its cleanup cannot finish.
    fixture.gap(&mut rng);
    fixture.intercept.arm(pred_address);
    fixture.operator_stops(&predecessor).await;
    let window = fixture.journal.len();
    fixture.converge(&predecessor).await;
    fixture.verdict(
        "E8-REMOVAL-REACHED (first stop)",
        fixture.refused_removals(pred_address, window) >= 1,
        &format!(
            "the first stop of {pred_alloc} did not reach remove_allocation_elements for its \
             source {pred_address}: the removal fault was never exercised"
        ),
    );

    // The hold: retries, other starts, and other stops in seeded order.
    let mut queue: VecDeque<HoldEvent> = events.into();
    let mut deferred_stops = 0_usize;
    while let Some(event) = queue.pop_front() {
        fixture.gap(&mut rng);
        match event {
            HoldEvent::FailingRetry => {
                let window = fixture.journal.len();
                fixture.converge(&predecessor).await;
                fixture.verdict(
                    "E8-REMOVAL-REACHED (retry)",
                    fixture.refused_removals(pred_address, window) >= 1,
                    &format!(
                        "a retried stop of {pred_alloc} did not re-run the removal for \
                         {pred_address}"
                    ),
                );
            }
            HoldEvent::SuccessorStart => {
                let workload = format!("e8s-{tag}-{successor_index}");
                successor_index += 1;
                let (alloc, _) = fixture.place_service(&workload).await;
                running.push_back((workload, alloc));
            }
            HoldEvent::SuccessorStop => {
                if let Some((workload, alloc)) = running.pop_front() {
                    fixture.stop_and_settle(&workload, &alloc).await;
                } else if deferred_stops < 1 {
                    // No other Service runs yet: stop the next one instead.
                    deferred_stops += 1;
                    queue.push_back(HoldEvent::SuccessorStop);
                } else {
                    fixture.note("hold: no running Service to stop; stop event dropped".to_owned());
                }
            }
        }
    }

    // The retry converges once the fault is disarmed.
    fixture.gap(&mut rng);
    fixture.intercept.disarm();
    for _ in 0..SETTLE_EVALUATIONS {
        fixture.converge(&predecessor).await;
        if fixture.released(&pred_alloc) {
            break;
        }
        fixture.gap(&mut rng);
    }

    // The next start after the release.
    fixture.gap(&mut rng);
    let next = format!("e8n-{tag}");
    let next_placement =
        if fixture.released(&pred_alloc) { Some(fixture.place_service(&next).await) } else { None };
    let (next_alloc, next_address) = next_placement.unzip();

    // --- Verdicts over the ordered journal. ---
    let entries = fixture.journal.entries();
    if let Some(index) =
        entries.iter().position(|entry| matches!(entry, Entry::Lease { alloc: None, .. }))
    {
        fixture.harness_failure(&format!("lease event #{index} carried no alloc field"));
    }
    let armed_at = entries
        .iter()
        .position(|entry| matches!(entry, Entry::Mark(Mark::FaultArmed)))
        .unwrap_or_else(|| unreachable!("the fault was armed before the first stop"));
    let disarmed_at = entries
        .iter()
        .position(|entry| matches!(entry, Entry::Mark(Mark::FaultDisarmed)))
        .unwrap_or_else(|| unreachable!("the fault was disarmed before the retry"));
    let retired: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| is_lease_event(entry, LEASE_RETIRED, &pred_alloc))
        .map(|(index, _)| index)
        .collect();
    let released: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| is_lease_event(entry, LEASE_RELEASED, &pred_alloc))
        .map(|(index, _)| index)
        .collect();
    let first_refused = entries.iter().position(|entry| {
        matches!(entry, Entry::Removal { source, refused: true } if *source == pred_address)
    });

    fixture.verdict(
        "E8-RETIRED-BEFORE-REMOVAL",
        first_refused.is_some_and(|refused| {
            retired.iter().any(|retired_at| *retired_at > armed_at && *retired_at < refused)
        }),
        &format!(
            "{LEASE_RETIRED} for {pred_alloc} at journal indices {retired:?} does not precede the \
             first refused removal of {pred_address} (index {first_refused:?}): the lease is \
             retired before the stop's first cleanup effect"
        ),
    );
    fixture.verdict(
        "E8-HELD-WHILE-FAILING",
        released.iter().all(|released_at| *released_at > disarmed_at),
        &format!(
            "{LEASE_RELEASED} for {pred_alloc} at journal indices {released:?} precedes the disarm \
             (index {disarmed_at}): a failed removal must keep the lease held"
        ),
    );

    let release_at = released.first().copied().unwrap_or(entries.len());
    let reassigned: Vec<String> = entries
        .iter()
        .take(release_at)
        .filter_map(|entry| match entry {
            Entry::Provision { alloc, address }
                if *address == pred_address && *alloc != pred_alloc =>
            {
                Some(format!("{alloc} at {address}"))
            }
            _ => None,
        })
        .collect();
    let hold_starts_seen = entries
        .iter()
        .take(release_at)
        .skip(armed_at)
        .filter(|entry| matches!(entry, Entry::Provision { .. }))
        .count();
    if hold_starts_seen == 0 {
        fixture.harness_failure(
            "no other start was provisioned while the predecessor's lease was held; the \
             no-reuse verdict would be vacuous",
        );
    }
    fixture.verdict(
        "E8-NO-REUSE-WHILE-HELD",
        reassigned.is_empty(),
        &format!(
            "while {pred_alloc}'s lease was held ({hold_starts_seen} other start(s) provisioned), \
             its address {pred_address} was assigned to {reassigned:?}"
        ),
    );

    let converging_removal =
        entries.iter().enumerate().skip(disarmed_at).find_map(|(index, entry)| {
            matches!(entry, Entry::Removal { source, refused: false } if *source == pred_address)
                .then_some(index)
        });
    fixture.verdict(
        "E8-RETRY-CONVERGES",
        released.len() == 1
            && converging_removal.is_some_and(|removed_at| removed_at < released[0]),
        &format!(
            "after the disarm (index {disarmed_at}) the retried stop of {pred_alloc} must remove its \
             members (successful removal at {converging_removal:?}) and then release its lease \
             exactly once; {LEASE_RELEASED} at {released:?}"
        ),
    );

    let spec_address = next_alloc.as_ref().and_then(|alloc| {
        fixture
            .driver
            .started_specs()
            .into_iter()
            .find(|spec| spec.alloc == *alloc)
            .and_then(|spec| spec.network)
            .map(|network| network.address)
    });
    fixture.verdict(
        "E8-REUSE-AFTER-RELEASE",
        next_address == Some(pred_address) && spec_address == Some(pred_address),
        &format!(
            "the first start after {pred_alloc}'s release ({next_alloc:?}) must receive its address \
             {pred_address}; its lease carried {next_address:?} and the spec the driver received \
             carried {spec_address:?}"
        ),
    );
    eprintln!("seed={seed} S-ND295-07 E8 schedule: GREEN");
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-07 — Stop removes the predecessor completely before its address is reused.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn a_failed_element_removal_keeps_the_address_until_a_retry_converges() {
    for seed in seeds() {
        eprintln!(
            "seed={seed} body=a_failed_element_removal_keeps_the_address_until_a_retry_converges"
        );
        retiring_cleanup_schedule(seed).await;
    }
}
