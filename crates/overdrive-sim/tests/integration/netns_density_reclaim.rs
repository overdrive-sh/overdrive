//! S-ND295-57 — leftover guest networks are reclaimed until released, across
//! restart predecessors, stopped workloads, and deleted workloads
//! (D-295-R11, FD § "[REF] Lifecycle action — row-neutral reclaim (D-295-R11) — ACCEPTED 2026-09-24" (emission through the validator)).
//!
//! # Contract under test
//!
//! `WorkloadLifecycle` emits `ReclaimAllocationNetwork` for every leased,
//! Failed or Terminated allocation no other action owns, on every return path;
//! a failing reclaim is re-dispatched no sooner than one second later, forever,
//! until the lease is released; the reclaim writes no row. At the cap
//! (`MAX_GUEST_NETWORK_ATTACHMENTS`) a predecessor whose restart is not yet due
//! is left to that restart; once due it is reclaimed first and its successor
//! is admitted only after the predecessor's lease is released (D-295-R7
//! recreate ordering).
//!
//! # Production owner path
//!
//! Every lifecycle decision runs through
//! `run_convergence_tick_with_guest_network_provisioner_for_test` (the
//! registered `WorkloadLifecycle`, the runtime `ViewStore`, and the production
//! action shim); the at-cap fill is `StartAllocation` actions through
//! `dispatch_with_guest_network_provisioner_for_test`. Only the guest-network
//! driven port is substituted, by `SimSharedGuestNetworkOwner` behind a
//! recording decorator that is the one owner instance. Crashes enter through
//! `SimDriver::inject_exit_after` and the production exit observer authors
//! the Failed rows. Operator stop and restart are the store effects of their
//! handlers; a deleted workload is its intent key removed through
//! `IntentStore::delete`, as the existing GC invariant drives the Absent
//! branch (no delete verb exists).
//!
//! # Fault
//!
//! `SimSharedGuestNetworkOwner::script_teardown_failure(true)` armed until each
//! leftover allocation has seen a seeded number of failed reclaim attempts,
//! then disarmed.
//!
//! # Oracle
//!
//! The decorator journals every `provision` and `teardown` with the sim-clock
//! instant and outcome; a test-local tracing `Layer` journals the pinned lease
//! events `guest_network.lease_released { alloc }` and
//! `guest_network.lease_retired { alloc }` (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the admission refusal projection and the lease events)) in the same order.
//! Reclaim re-dispatch instants per allocation are at least one second apart
//! and within one evaluation step of that second; each leftover lease is
//! released once, after the disarm, and never touched again; the leftover
//! allocation's row is byte-equal before and after its reclaims.
//!
//! # Lane and time
//!
//! Integration binary. The at-cap body reaches the fixed
//! `MAX_GUEST_NETWORK_ATTACHMENTS` = 16,384 cap (D-295-R6) by admitting
//! filler allocations through the production action shim for every seed; the
//! cap is a constant, so no smaller node can exercise it. At RED the fill
//! dispatches in batches and is cheap (the whole at-cap body reached its RED
//! verdict in 0.8 s, `red-classification.md` Phase G, run G4-L05), but its cost once
//! DELIVER 07-03 makes the at-cap reconcile evaluations run is unmeasured, so
//! the body stays out of the 60 s default lane with a widened nextest budget
//! until 07-03 records its GREEN run time (a 07-03 review item: move it back
//! to the acceptance binary if that time fits the default lane). The
//! every-path body shares this file's seam fixture. No wall time is read: the only clock is the
//! fixture's `SimClock`, advanced by the harness, and every wait yields to the
//! current-thread runtime, so each trajectory is a function of its seed.
//!
//! Reproduce with `OVERDRIVE_ND295_RECLAIM_SEEDS=<seed>[,<seed>…] cargo xtask
//! lima run -- cargo nextest run -p overdrive-sim --test integration
//! --features integration-tests --run-ignored ignored-only --no-capture
//! -E 'test(/netns_density_reclaim/)'`.

#![allow(
    clippy::print_stderr,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::large_futures,
    reason = "seeded acceptance harness: every verdict prints its seed; fixture preconditions fail loudly"
)]

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use overdrive_control_plane::action_shim::{
    ShimError, dispatch_with_guest_network_provisioner_for_test,
};
use overdrive_control_plane::guest_network::{
    GuestAddressPool, GuestNetworkPlan, GuestNetworkProvisioner, Result as GuestNetworkResult,
    SharedGuestNetworkAudit, SharedGuestNetworkAuditError, SharedGuestNetworkOwner, TapActivation,
    TapQuiescence,
};
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, run_convergence_tick_with_guest_network_provisioner_for_test,
};
use overdrive_control_plane::worker::exit_observer;
use overdrive_control_plane::{AppState, workload_lifecycle};
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, Service, VmInput, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::{ListenerInput, ServiceSpecInput};
use overdrive_core::guest_network::{
    GuestNetworkExecGate, GuestNetworkExecSupervisor, GuestNetworkExecWiring,
    MAX_GUEST_NETWORK_ATTACHMENTS,
};
use overdrive_core::id::{AllocationId, MeshServiceName, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, ReconcilerName, TargetResource, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationSpec, DriverPayload, DriverType, ExitKind, Resources, VmPayload,
};
use overdrive_core::traits::intent_store::{IntentStore, PutOutcome};
use overdrive_core::traits::mtls_enforcement::MtlsLimits;
use overdrive_core::traits::mtls_resolve::MtlsResolution;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::backoff_for_attempt;
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

const SEEDS_ENV: &str = "OVERDRIVE_ND295_RECLAIM_SEEDS";
const DEFAULT_SEEDS: [u64; 3] =
    [0x0295_0057_0000_0001, 0x0295_0057_0000_0002, 0x0295_0057_0000_0003];
/// The Phase-1 single-node baseline id (`workload_lifecycle::baseline_nodes_phase1`).
const NODE: &str = "local";
/// Today's action-pool constants (`guest_network::action_pool`).
const GUEST_PREFIX: &str = "100.95.0.0/16";
const GUEST_BRIDGE: &str = "ovd-gbr0";
const GUEST_GATEWAY: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
/// The reclaim re-dispatch spacing of D-295-R11: `backoff_for_attempt`,
/// constant one second until GH #137 (FD § "[REF] Lifecycle action — row-neutral reclaim (D-295-R11) — ACCEPTED 2026-09-24" (real cadence and load: the cadence); TS S-ND295-57 oracle).
const RECLAIM_SPACING: Duration = Duration::from_secs(1);
/// Evaluation steps of simulated time; every leftover target is evaluated
/// once per step, so a due reclaim runs within one step of its deadline.
const STEP_MIN_MS: u64 = 100;
const STEP_MAX_MS: u64 = 700;
/// Simulated-time budget after the fault is armed.
const RECLAIM_BUDGET: Duration = Duration::from_secs(30);
/// Simulated time the loop keeps evaluating after every lease is released.
const SETTLE_AFTER_RELEASE: Duration = Duration::from_secs(3);
/// Bounded polls while waiting for the exit observer's Failed rows.
const CRASH_POLL_BUDGET: usize = 2_000;
/// `StartAllocation` actions per fill dispatch.
const FILL_BATCH: usize = 512;
const LEASE_RELEASED: &str = "guest_network.lease_released";
const LEASE_RETIRED: &str = "guest_network.lease_retired";

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

// ---------------------------------------------------------------------------
// Ordered observation journal (driven port + lease events + test marks).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    Armed,
    Disarmed,
}

#[derive(Debug, Clone)]
enum Entry {
    Provision {
        alloc: AllocationId,
    },
    Teardown {
        alloc: AllocationId,
        at: UnixInstant,
        ok: bool,
    },
    /// A pinned lease event; `alloc` is the rendered `alloc` field.
    Lease {
        name: String,
        alloc: String,
    },
    Mark(Mark),
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

    fn mark_index(&self, mark: Mark) -> Option<usize> {
        self.0.lock().iter().position(|entry| matches!(entry, Entry::Mark(found) if *found == mark))
    }

    /// Journal indices of the lease event `name` naming `alloc`.
    fn lease_events(&self, name: &str, alloc: &AllocationId) -> Vec<usize> {
        self.0
            .lock()
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                matches!(entry, Entry::Lease { name: found, alloc: named }
                    if found == name && named.contains(alloc.as_str()))
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// `(journal index, instant, ok)` of every teardown of `alloc` at or after `from`.
    fn teardowns(&self, alloc: &AllocationId, from: usize) -> Vec<(usize, UnixInstant, bool)> {
        self.0
            .lock()
            .iter()
            .enumerate()
            .skip(from)
            .filter_map(|(index, entry)| match entry {
                Entry::Teardown { alloc: torn, at, ok } if torn == alloc => Some((index, *at, *ok)),
                _ => None,
            })
            .collect()
    }

    /// Journal indices of every provision after `from` of an allocation other
    /// than `except` whose id starts with `prefix`.
    fn provisions_after(&self, from: usize, prefix: &str, except: &AllocationId) -> Vec<usize> {
        self.0
            .lock()
            .iter()
            .enumerate()
            .skip(from)
            .filter(|(_, entry)| {
                matches!(entry, Entry::Provision { alloc, .. }
                    if alloc != except && alloc.as_str().starts_with(prefix))
            })
            .map(|(index, _)| index)
            .collect()
    }
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

/// Test-local `Layer` journaling the pinned `guest_network.lease_*` events.
/// Thread-local capture suffices: the runtime is current-thread.
struct LeaseCapture(Journal);

impl<S> Layer<S> for LeaseCapture
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let name = event.metadata().name();
        let interesting = name.starts_with("guest_network.lease_") || name.starts_with("event ");
        if !interesting {
            return;
        }
        let mut fields = BTreeMap::new();
        event.record(&mut FieldText(&mut fields));
        let named = if name.starts_with("guest_network.lease_") {
            Some(name.to_owned())
        } else {
            fields
                .get("message")
                .map(|message| message.trim_matches('"').to_owned())
                .filter(|message| message.starts_with("guest_network.lease_"))
        };
        if let Some(name) = named {
            let alloc = fields.get("alloc").cloned().unwrap_or_default();
            self.0.push(Entry::Lease { name, alloc });
        }
    }
}

// ---------------------------------------------------------------------------
// The one owner instance: the sim owner behind a recording decorator.
// ---------------------------------------------------------------------------

/// Journals every `provision` and `teardown` with the sim-clock instant and
/// outcome, and delegates everything to `SimSharedGuestNetworkOwner`. It
/// observes only; it authors no lease, row, or decision.
struct RecordingOwner {
    inner: SimSharedGuestNetworkOwner,
    clock: Arc<SimClock>,
    journal: Journal,
}

impl RecordingOwner {
    fn now(&self) -> UnixInstant {
        UnixInstant::from_clock(self.clock.as_ref())
    }
}

#[async_trait::async_trait]
impl GuestNetworkProvisioner for RecordingOwner {
    async fn provision(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        self.journal.push(Entry::Provision { alloc: plan.alloc().clone() });
        self.inner.provision(plan).await
    }

    async fn activate(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<TapActivation> {
        self.inner.activate(plan).await
    }

    async fn teardown(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        let outcome = self.inner.teardown(plan).await;
        self.journal.push(Entry::Teardown {
            alloc: plan.alloc().clone(),
            at: self.now(),
            ok: outcome.is_ok(),
        });
        outcome
    }
}

#[async_trait::async_trait]
impl SharedGuestNetworkOwner for RecordingOwner {
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

// ---------------------------------------------------------------------------
// Seam fixture (TS § Seam fixture).
// ---------------------------------------------------------------------------

struct Fixture {
    _tmp: TempDir,
    seed: u64,
    state: AppState,
    clock: Arc<SimClock>,
    driver: Arc<SimDriver>,
    owner: Arc<RecordingOwner>,
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
    async fn compose(seed: u64) -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let mut runtime = ReconcilerRuntime::new(tmp.path(), Arc::new(SimViewStore::new()))
            .expect("reconciler runtime");
        runtime.register(workload_lifecycle()).await.expect("register workload-lifecycle");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("intent store"));
        let node_id = NodeId::new(NODE).expect("node id");
        let obs: Arc<dyn ObservationStore> =
            Arc::new(SimObservationStore::single_peer(node_id.clone(), seed));
        let clock = Arc::new(SimClock::new());
        let driver = Arc::new(SimDriver::with_clock(DriverType::Vm, clock.clone()));
        let journal = Journal::default();

        // The one owner instance: the seams' `provisioner` and AppState's owner.
        let owner = Arc::new(RecordingOwner {
            inner: SimSharedGuestNetworkOwner::default(),
            clock: Arc::clone(&clock),
            journal: journal.clone(),
        });
        // One EXEC wiring over the fixture clock; the gate starts BootClosed
        // and only the kept supervisor moves it.
        let wiring = GuestNetworkExecWiring::new(clock.clone() as Arc<dyn Clock>);
        let gate = wiring.gate();
        let supervisor = wiring.supervisor();
        // One pool from the doc-hidden constructor with today's constants.
        let pool = Arc::new(GuestAddressPool::new(
            GUEST_PREFIX.parse().expect("static guest prefix"),
            GUEST_BRIDGE.to_owned(),
            GUEST_GATEWAY,
            GUEST_GATEWAY,
        ));
        // One worker over the sim mTLS ports, its shared owner started.
        let worker = Arc::new(MtlsInterceptWorker::new(
            Arc::new(SimMtlsEnforcement::new(
                Arc::new(SimIdentityRead::new(BTreeMap::new(), None)),
                MtlsLimits::default(),
            )),
            Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh)),
            clock.clone() as Arc<dyn Clock>,
            Arc::new(SimMtlsIntercept::new()),
        ));
        worker.start_shared_owner().await.expect("worker shared owner starts over sim ports");

        let allocator =
            overdrive_control_plane::test_default_allocator(store.clone() as Arc<dyn IntentStore>);
        // DELIVER 05-01 changes this one call: the pinned `AppState::new`
        // appends `worker`, `owner`, `gate`, and `pool` as required parameters
        // (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (how the gate reaches the shim, and the `AppState` constructors)). Until then the fixture keeps them for its bodies.
        let state = AppState::new(
            store,
            store_path,
            obs,
            Arc::new(runtime),
            driver.clone(),
            clock.clone() as Arc<dyn Clock>,
            Arc::new(SimDataplane::new()),
            Arc::new(SimCa::new(Arc::new(SimEntropy::new(seed)))),
            Arc::new(IdentityMgr::new(None)),
            node_id,
            allocator,
            overdrive_control_plane::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
        );
        // The production exit observer authors a crashed allocation's Failed
        // row from the driver's exit event.
        exit_observer::spawn(
            state.obs.clone(),
            driver.clone(),
            state.lifecycle_events.clone(),
            clock.clone(),
        );
        Self {
            _tmp: tmp,
            seed,
            state,
            clock,
            driver,
            owner,
            supervisor,
            gate,
            pool,
            worker,
            journal,
            reconciler: ReconcilerName::new("workload-lifecycle").expect("reconciler name"),
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

    fn trace(&self) -> String {
        self.notes.lock().join("\n  ")
    }

    fn harness_failure(&self, what: &str) -> ! {
        panic!(
            "seed={}: harness precondition failed (not a contract verdict): {what}\n\
             trace:\n  {}\n\
             reproduce: {SEEDS_ENV}={} cargo xtask lima run -- cargo nextest run -p overdrive-sim \
             --test integration --features integration-tests --run-ignored ignored-only \
             --no-capture -E 'test(/netns_density_reclaim/)'",
            self.seed,
            self.trace(),
            self.seed
        );
    }

    /// The gate every activation claims: opened once, as `run_server*`'s boot
    /// sequence does (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (gate state outside `run_server*`)).
    fn open_gate(&self) {
        if !self.supervisor.open_after_boot() {
            self.harness_failure("open_after_boot was refused");
        }
    }

    fn unix_now(&self) -> UnixInstant {
        UnixInstant::from_clock(self.clock.as_ref())
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
            listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
            startup_probes: Vec::new(),
            readiness_probes: Vec::new(),
            liveness_probes: Vec::new(),
        })
        .expect("valid service spec");
        let intent = WorkloadIntent::Service(service.clone());
        let archived = intent.archive_for_store().expect("archive workload intent");
        let digest = intent.spec_digest().expect("spec digest");
        if let Ok(name) =
            MeshServiceName::new(&format!("{}.{}", service.id.as_str(), MeshServiceName::SUFFIX))
        {
            self.state.frontend_addr_allocator.assign(&name).expect("frontend address");
        }
        let vip = {
            let mut guard = self.state.allocator.lock().await;
            guard.allocate(*digest.as_bytes()).await.expect("service vip")
        };
        let key = IntentKey::for_workload(&service.id);
        let outcome = self
            .state
            .store
            .put_if_absent(key.as_bytes(), archived.as_ref())
            .await
            .expect("intent write");
        if !matches!(outcome, PutOutcome::Inserted) {
            self.harness_failure(&format!("service {workload} was not fresh"));
        }
        let kind_key = IntentKey::for_workload_kind(&service.id);
        self.state
            .store
            .put(kind_key.as_bytes(), &[WorkloadKind::Service.discriminator_byte()])
            .await
            .expect("kind write");
        self.state.listener_facts.lock().await.upsert(service.id.clone(), &vip, &service.listeners);
        self.note(format!("operator deploys service {workload}"));
    }

    /// Store effect of `handlers::stop_workload`.
    async fn operator_stops(&self, workload: &str) {
        let id = WorkloadId::new(workload).expect("workload id");
        let stop_key = IntentKey::for_workload_stop(&id);
        self.state.store.put_if_absent(stop_key.as_bytes(), b"").await.expect("stop write");
        self.note(format!("operator stops {workload}"));
    }

    /// The workload's intent is withdrawn (the Absent/GC branch input, as the
    /// existing `workload_gc_absent_intent` invariant drives it).
    async fn intent_deleted(&self, workload: &str) {
        let id = WorkloadId::new(workload).expect("workload id");
        let key = IntentKey::for_workload(&id);
        self.state.store.delete(key.as_bytes()).await.expect("intent delete");
        self.note(format!("intent of {workload} deleted"));
    }

    /// One production `workload-lifecycle` evaluation for `workload`.
    async fn converge(&self, workload: &str) -> Result<(), String> {
        let n = self.tick.fetch_add(1, Ordering::SeqCst);
        let now = self.clock.now();
        let target = TargetResource::new(&format!("workload/{workload}")).expect("target");
        let outcome = Box::pin(run_convergence_tick_with_guest_network_provisioner_for_test(
            &self.state,
            &self.reconciler,
            &target,
            now,
            n,
            now + Duration::from_secs(1),
            self.owner.as_ref(),
        ))
        .await
        .map_err(|error| format!("{error:?}"));
        if let Err(error) = &outcome {
            self.note(format!("evaluation of {workload} at {}: {error}", self.unix_now()));
        }
        outcome
    }

    async fn dispatch(&self, actions: Vec<Action>) -> Result<(), ShimError> {
        let now = self.clock.now();
        let tick = TickContext {
            now,
            now_unix: self.unix_now(),
            tick: self.tick.fetch_add(1, Ordering::SeqCst),
            deadline: now + Duration::from_secs(1),
        };
        Box::pin(dispatch_with_guest_network_provisioner_for_test(
            actions,
            &self.state,
            &tick,
            self.owner.as_ref(),
        ))
        .await
    }

    async fn row(&self, alloc: &AllocationId) -> Option<AllocStatusRow> {
        self.state.obs.alloc_status_row(alloc).await.expect("alloc status row read")
    }

    async fn running_alloc(&self, workload: &str) -> AllocationId {
        let rows = self.state.obs.alloc_status_rows().await.expect("alloc status rows");
        rows.into_iter()
            .find(|row| row.workload_id.as_str() == workload && row.state == AllocState::Running)
            .map_or_else(
                || self.harness_failure(&format!("{workload} has no Running allocation")),
                |row| row.alloc_id,
            )
    }

    /// Deploy and place a Service through the production evaluation.
    async fn place_service(&self, workload: &str) -> AllocationId {
        self.operator_deploys_service(workload).await;
        if let Err(error) = self.converge(workload).await {
            self.harness_failure(&format!("placement of {workload} failed: {error}"));
        }
        let alloc = self.running_alloc(workload).await;
        if self
            .journal
            .provisions_after(0, &format!("alloc-{workload}-"), &placeholder())
            .is_empty()
        {
            self.harness_failure(&format!("{workload} acquired no attachment"));
        }
        alloc
    }

    /// Crash allocations through the driver exit port; the production exit
    /// observer authors their Failed rows. No row is seeded.
    async fn crash(&self, allocs: &[AllocationId]) {
        for alloc in allocs {
            self.driver.inject_exit_after(
                alloc,
                Duration::ZERO,
                ExitKind::Crashed { exit_code: None, signal: Some(9) },
            );
        }
        // Every step yields to the current-thread runtime (the exit-injection
        // tasks and the exit observer run on it) and advances only the sim
        // clock; no wall time is read, so the step count is a function of the
        // seed and the tasks' own await points.
        for _ in 0..CRASH_POLL_BUDGET {
            tokio::task::yield_now().await;
            self.clock.tick(Duration::from_millis(1));
            let mut failed = 0;
            for alloc in allocs {
                if self.row(alloc).await.is_some_and(|row| row.state == AllocState::Failed) {
                    failed += 1;
                }
            }
            if failed == allocs.len() {
                self.note(format!("crashed {allocs:?} -> Failed rows by the exit observer"));
                return;
            }
        }
        self.harness_failure(&format!(
            "exit observer never published Failed for {allocs:?} within {CRASH_POLL_BUDGET} \
             yield-and-tick steps"
        ));
    }

    fn arm_teardown_failure(&self) {
        self.owner.inner.script_teardown_failure(true);
        self.journal.push(Entry::Mark(Mark::Armed));
        self.note(format!("teardown failure armed at {}", self.unix_now()));
    }

    fn disarm_teardown_failure(&self) {
        self.owner.inner.script_teardown_failure(false);
        self.journal.push(Entry::Mark(Mark::Disarmed));
        self.note(format!("teardown failure disarmed at {}", self.unix_now()));
    }

    fn step(&self, rng: &mut StdRng) -> Duration {
        let step = Duration::from_millis(rng.gen_range(STEP_MIN_MS..=STEP_MAX_MS));
        self.clock.tick(step);
        step
    }
}

/// An allocation id no workload uses, for "any other allocation" filters.
fn placeholder() -> AllocationId {
    AllocationId::new("alloc-nd295-none-0").expect("placeholder allocation id")
}

fn tag(seed: u64) -> String {
    format!("{seed:016x}")
}

/// One leftover allocation under observation.
struct Leftover {
    label: &'static str,
    alloc: AllocationId,
    /// Journal index from which this allocation's teardowns are reclaims (its
    /// owner's own cleanup attempt, if any, precedes it).
    reclaims_from: Option<usize>,
    /// The row when the allocation became leftover.
    row: Option<AllocStatusRow>,
}

impl Leftover {
    /// Instants and outcomes of every reclaim teardown so far.
    fn reclaims(&self, journal: &Journal) -> Vec<(usize, UnixInstant, bool)> {
        self.reclaims_from.map_or_else(Vec::new, |from| journal.teardowns(&self.alloc, from))
    }
}

/// Reclaim instants of one allocation are at least [`RECLAIM_SPACING`]
/// apart, and a due reclaim runs within one evaluation step of its deadline.
fn assert_reclaim_cadence(seed: u64, leftover: &Leftover, journal: &Journal) {
    let reclaims = leftover.reclaims(journal);
    for pair in reclaims.windows(2) {
        let gap = pair[1].1.as_unix_duration().saturating_sub(pair[0].1.as_unix_duration());
        assert!(
            gap >= RECLAIM_SPACING,
            "seed={seed}: {} {} reclaim re-dispatched {gap:?} after the previous attempt; the \
             reclaim backoff spaces re-dispatch at least {RECLAIM_SPACING:?} apart (D-295-R11); \
             reclaims {reclaims:?}",
            leftover.label,
            leftover.alloc,
        );
        assert!(
            gap <= RECLAIM_SPACING + Duration::from_millis(STEP_MAX_MS),
            "seed={seed}: {} {} reclaim was re-dispatched {gap:?} after the previous attempt; \
             it is retried at the one-second cadence (within one evaluation step of \
             {STEP_MAX_MS} ms); reclaims {reclaims:?}",
            leftover.label,
            leftover.alloc,
        );
    }
}

/// Released once, after the disarm; never torn down again; the row the
/// allocation had when it became leftover is unchanged.
async fn assert_released_once_without_row_write(
    fixture: &Fixture,
    leftover: &Leftover,
    disarmed_at: usize,
) {
    let seed = fixture.seed;
    let released = fixture.journal.lease_events(LEASE_RELEASED, &leftover.alloc);
    assert!(
        released.len() == 1 && released[0] > disarmed_at,
        "seed={seed}: {} {} expected exactly one {LEASE_RELEASED} after the disarm (journal index \
         {disarmed_at}); got {released:?}",
        leftover.label,
        leftover.alloc,
    );
    let after_release: Vec<_> = fixture.journal.teardowns(&leftover.alloc, released[0]);
    assert!(
        after_release.is_empty(),
        "seed={seed}: {} {} was torn down again after its lease was released: {after_release:?}",
        leftover.label,
        leftover.alloc,
    );
    let row = fixture.row(&leftover.alloc).await;
    assert_eq!(
        row, leftover.row,
        "seed={seed}: a reclaim writes no row; {} {} row changed while it was reclaimed",
        leftover.label, leftover.alloc,
    );
}

// ---------------------------------------------------------------------------
// Bodies.
// ---------------------------------------------------------------------------

async fn leftover_networks_on_every_path(seed: u64) {
    let mut rng = StdRng::seed_from_u64(seed);
    let failures = rng.gen_range(1..=3_usize);
    eprintln!(
        "seed={seed} S-ND295-57 every-path: failing reclaim attempts before disarm={failures}"
    );
    let fixture = Fixture::compose(seed).await;
    let _capture = fixture.capture();
    fixture.open_gate();

    let restarted = format!("r57-{}", tag(seed));
    let stopped = format!("s57-{}", tag(seed));
    let deleted = format!("d57-{}", tag(seed));
    let pred_restarted = fixture.place_service(&restarted).await;
    let pred_stopped = fixture.place_service(&stopped).await;
    let pred_deleted = fixture.place_service(&deleted).await;
    fixture.crash(&[pred_restarted.clone(), pred_stopped.clone(), pred_deleted.clone()]).await;

    // The stopped and deleted workloads' crashed allocations become leftover
    // at once; the restart predecessor becomes leftover after its restart's
    // one cleanup attempt fails.
    fixture.operator_stops(&stopped).await;
    fixture.intent_deleted(&deleted).await;
    fixture.arm_teardown_failure();
    let armed_at = fixture.journal.mark_index(Mark::Armed).unwrap_or(0);
    let mut leftovers = vec![
        Leftover {
            label: "restart predecessor",
            alloc: pred_restarted.clone(),
            reclaims_from: None,
            row: None,
        },
        Leftover {
            label: "stopped workload",
            alloc: pred_stopped.clone(),
            reclaims_from: Some(armed_at),
            row: fixture.row(&pred_stopped).await,
        },
        Leftover {
            label: "deleted workload",
            alloc: pred_deleted.clone(),
            reclaims_from: Some(armed_at),
            row: fixture.row(&pred_deleted).await,
        },
    ];
    let workloads = [restarted.clone(), stopped.clone(), deleted.clone()];
    let successor_prefix = format!("alloc-{restarted}-");

    let started = fixture.unix_now().as_unix_duration();
    let mut released_at: Option<Duration> = None;
    loop {
        let elapsed = fixture.unix_now().as_unix_duration().saturating_sub(started);
        if elapsed > RECLAIM_BUDGET {
            let counts: Vec<String> = leftovers
                .iter()
                .map(|leftover| {
                    format!(
                        "{} {}: reclaims {:?}, released {:?}",
                        leftover.label,
                        leftover.alloc,
                        leftover.reclaims(&fixture.journal),
                        fixture.journal.lease_events(LEASE_RELEASED, &leftover.alloc),
                    )
                })
                .collect();
            panic!(
                "seed={seed}: leftover leases were not reclaimed until released within \
                 {RECLAIM_BUDGET:?} of simulated time (failing attempts before disarm={failures}, \
                 disarmed={:?}):\n  {}\ntrace:\n  {}",
                fixture.journal.mark_index(Mark::Disarmed),
                counts.join("\n  "),
                fixture.trace(),
            );
        }
        fixture.step(&mut rng);
        let mut order = workloads.clone();
        order.shuffle(&mut rng);
        for workload in &order {
            let before = fixture.journal.len();
            // Evaluation errors are observed through the journal, not here.
            fixture.converge(workload).await.unwrap_or(());
            if *workload == restarted && leftovers[0].reclaims_from.is_none() {
                let successors =
                    fixture.journal.provisions_after(before, &successor_prefix, &pred_restarted);
                if !successors.is_empty() {
                    // This evaluation was the restart; its one predecessor
                    // cleanup attempt ends with it.
                    leftovers[0].reclaims_from = Some(fixture.journal.len());
                    leftovers[0].row = fixture.row(&pred_restarted).await;
                }
            }
        }

        let disarmed = fixture.journal.mark_index(Mark::Disarmed).is_some();
        if !disarmed
            && leftovers.iter().all(|leftover| {
                leftover.reclaims(&fixture.journal).iter().filter(|(_, _, ok)| !ok).count()
                    >= failures
            })
        {
            fixture.disarm_teardown_failure();
        }
        let all_released = disarmed
            && leftovers.iter().all(|leftover| {
                !fixture.journal.lease_events(LEASE_RELEASED, &leftover.alloc).is_empty()
            });
        let now = fixture.unix_now().as_unix_duration();
        match released_at {
            None if all_released => released_at = Some(now),
            Some(at) if now.saturating_sub(at) >= SETTLE_AFTER_RELEASE => break,
            _ => {}
        }
    }

    let disarmed_at = fixture.journal.mark_index(Mark::Disarmed).unwrap_or(usize::MAX);
    for leftover in &leftovers {
        let reclaims = leftover.reclaims(&fixture.journal);
        let failed = reclaims.iter().filter(|(_, _, ok)| !ok).count();
        assert!(
            failed >= failures && reclaims.last().is_some_and(|(_, _, ok)| *ok),
            "seed={seed}: {} {} expected at least {failures} failing reclaims then one that \
             succeeds; got {reclaims:?}",
            leftover.label,
            leftover.alloc,
        );
        assert_reclaim_cadence(seed, leftover, &fixture.journal);
        assert_released_once_without_row_write(&fixture, leftover, disarmed_at).await;
    }
    eprintln!("seed={seed} S-ND295-57 every-path: GREEN");
}

/// A Job allocation of a workload with no intent, admitted through the
/// production action owner only to occupy one attachment.
fn filler_action(tag: &str, index: usize) -> Action {
    let workload = format!("f57-{tag}-{index:05}");
    let alloc = AllocationId::new(&format!("alloc-{workload}-0")).expect("allocation id");
    Action::StartAllocation {
        alloc_id: alloc.clone(),
        workload_id: WorkloadId::new(&workload).expect("workload id"),
        node_id: NodeId::new(NODE).expect("node id"),
        spec: AllocationSpec {
            alloc,
            identity: SpiffeId::new(&format!(
                "spiffe://overdrive.local/workload/{workload}/alloc/{workload}-0"
            ))
            .expect("SPIFFE ID"),
            driver: DriverPayload::Vm(VmPayload {
                command: "/bin/sleep".to_owned(),
                args: vec!["3600".to_owned()],
                kernel: PathBuf::from("/nd295/kernel"),
                rootfs: PathBuf::from("/nd295/rootfs.ext4"),
            }),
            resources: Resources { cpu_milli: 0, memory_bytes: 256 * 1024 },
            probe_descriptors: Vec::new(),
            network: None,
            service_ports: Vec::new(),
        },
        kind: WorkloadKind::Job,
    }
}

/// While its restart is not yet due, a predecessor at the cap is left to that
/// restart: no reclaim teardown, no retirement, no successor admission after
/// journal index `from`.
fn assert_left_to_restart(
    fixture: &Fixture,
    pred: &AllocationId,
    due: UnixInstant,
    successor_prefix: &str,
    from: usize,
) {
    let torn = fixture.journal.teardowns(pred, 0);
    let retired = fixture.journal.lease_events(LEASE_RETIRED, pred);
    let successors = fixture.journal.provisions_after(from, successor_prefix, pred);
    assert!(
        torn.is_empty() && retired.is_empty() && successors.is_empty(),
        "seed={}: at the cap, {pred} whose restart is not yet due (due {due}, now {}) is left to \
         its restart: no reclaim, no retirement, no successor; teardowns {torn:?}, retired \
         {retired:?}, successor provisions {successors:?}",
        fixture.seed,
        fixture.unix_now(),
    );
}

async fn not_yet_due_restart_at_the_cap(seed: u64) {
    let mut rng = StdRng::seed_from_u64(seed);
    let failures = rng.gen_range(1..=3_usize);
    let cap = usize::try_from(MAX_GUEST_NETWORK_ATTACHMENTS).expect("cap fits usize");
    eprintln!(
        "seed={seed} S-ND295-57 at-cap: cap={cap} failing reclaim attempts before disarm={failures}"
    );
    let fixture = Fixture::compose(seed).await;
    let _capture = fixture.capture();
    fixture.open_gate();
    let tag = tag(seed);

    // Fill the node to two below the cap through the production action
    // owner: every filler lease comes from the shim's own admission.
    let fillers: Vec<Action> = (0..cap - 2).map(|index| filler_action(&tag, index)).collect();
    for (batch_index, batch) in fillers.chunks(FILL_BATCH).enumerate() {
        if let Err(error) = fixture.dispatch(batch.to_vec()).await {
            fixture.harness_failure(&format!("fill batch {batch_index} failed: {error:?}"));
        }
    }
    let filled = fixture
        .journal
        .entries()
        .iter()
        .filter(|entry| matches!(entry, Entry::Provision { .. }))
        .count();
    if filled != cap - 2 {
        fixture
            .harness_failure(&format!("fill acquired {filled} attachments, expected {}", cap - 2));
    }
    eprintln!("seed={seed} fill: {filled} attachments");

    // A Service below the cap crashes and is restarted with room: its
    // successor carries the restart backoff (only a successor's failure has a
    // not-yet-due restart), and the restart's own cleanup releases the
    // original allocation.
    let victim = format!("v57-{tag}");
    let successor_prefix = format!("alloc-{victim}-");
    let original = fixture.place_service(&victim).await;
    fixture.crash(std::slice::from_ref(&original)).await;
    let restart_from = fixture.journal.len();
    fixture.converge(&victim).await.unwrap_or(());
    let pred = fixture.running_alloc(&victim).await;
    if pred == original
        || fixture.journal.provisions_after(restart_from, &successor_prefix, &original).is_empty()
        || !fixture.journal.teardowns(&original, restart_from).iter().any(|(_, _, ok)| *ok)
    {
        fixture.harness_failure(&format!(
            "the below-cap restart of {original} did not admit a successor and clean up its \
             predecessor"
        ));
    }
    // One more filler takes the attachment the original released: the node is
    // at the cap with the successor admitted.
    if let Err(error) = fixture.dispatch(vec![filler_action(&tag, cap - 2)]).await {
        fixture.harness_failure(&format!("final filler failed: {error:?}"));
    }
    let target = TargetResource::new(&format!("workload/{victim}")).expect("target");
    let view = fixture.state.runtime.view_for_workload_lifecycle(&target);
    let Some(seen_at) = view.last_failure_seen_at.get(&pred).copied() else {
        fixture.harness_failure(&format!("the successor {pred} carries no restart backoff input"));
    };
    let attempts = view.restart_counts.get(&pred).copied().unwrap_or(0);
    let due = seen_at + backoff_for_attempt(attempts);

    // The successor crashes inside its backoff window, at the cap.
    fixture.crash(std::slice::from_ref(&pred)).await;
    let crashed_row = fixture.row(&pred).await;
    if fixture.unix_now() >= due {
        fixture.harness_failure(&format!("{pred} crashed after its restart was already due"));
    }
    eprintln!("seed={seed} at-cap: {pred} failed at {}, restart due at {due}", fixture.unix_now());
    let phase_from = fixture.journal.len();

    // Not yet due, at the cap: the predecessor is left to its restart.
    fixture.converge(&victim).await.unwrap_or(());
    assert_left_to_restart(&fixture, &pred, due, &successor_prefix, phase_from);
    loop {
        let step = Duration::from_millis(rng.gen_range(STEP_MIN_MS..=STEP_MAX_MS));
        if fixture.unix_now() + step >= due {
            break;
        }
        fixture.clock.tick(step);
        fixture.converge(&victim).await.unwrap_or(());
        assert_left_to_restart(&fixture, &pred, due, &successor_prefix, phase_from);
    }

    // Due at the cap: the predecessor is reclaimed first; its successor is
    // admitted only after the predecessor's lease is released.
    fixture.arm_teardown_failure();
    let armed_at = fixture.journal.mark_index(Mark::Armed).unwrap_or(0);
    let now = fixture.unix_now().as_unix_duration();
    fixture.clock.tick(due.as_unix_duration().saturating_sub(now));
    let leftover = Leftover {
        label: "at-cap predecessor",
        alloc: pred.clone(),
        reclaims_from: Some(armed_at),
        row: crashed_row,
    };
    let loop_started = fixture.unix_now().as_unix_duration();
    let successor_at = loop {
        let elapsed = fixture.unix_now().as_unix_duration().saturating_sub(loop_started);
        assert!(
            elapsed <= RECLAIM_BUDGET,
            "seed={seed}: at the cap, {pred} was not reclaimed and replaced within \
             {RECLAIM_BUDGET:?} of simulated time after its restart was due; reclaims {:?}, \
             released {:?}, disarmed {:?}\ntrace:\n  {}",
            leftover.reclaims(&fixture.journal),
            fixture.journal.lease_events(LEASE_RELEASED, &pred),
            fixture.journal.mark_index(Mark::Disarmed),
            fixture.trace(),
        );
        fixture.converge(&victim).await.unwrap_or(());
        let released = fixture.journal.lease_events(LEASE_RELEASED, &pred);
        let successors = fixture.journal.provisions_after(armed_at, &successor_prefix, &pred);
        if let Some(first_successor) = successors.first().copied() {
            assert!(
                released.first().is_some_and(|release| *release < first_successor),
                "seed={seed}: at the cap, a successor of {victim} was admitted (journal index \
                 {first_successor}) while its predecessor {pred} still held its lease (released at \
                 {released:?}); the held population would exceed {cap}",
            );
            break first_successor;
        }
        if !released.is_empty() {
            // Between the release and the restart: the reclaim wrote no row.
            let row = fixture.row(&pred).await;
            assert_eq!(
                row, leftover.row,
                "seed={seed}: a reclaim writes no row; the at-cap predecessor {pred} row changed"
            );
        }
        if fixture.journal.mark_index(Mark::Disarmed).is_none()
            && leftover.reclaims(&fixture.journal).iter().filter(|(_, _, ok)| !ok).count()
                >= failures
        {
            fixture.disarm_teardown_failure();
        }
        fixture.step(&mut rng);
    };

    let disarmed_at = fixture.journal.mark_index(Mark::Disarmed).unwrap_or(usize::MAX);
    let reclaims = leftover.reclaims(&fixture.journal);
    assert!(
        reclaims.iter().all(|(_, at, _)| *at >= due),
        "seed={seed}: no reclaim of {pred} before its restart was due at {due}: {reclaims:?}"
    );
    assert!(
        reclaims.iter().filter(|(_, _, ok)| !ok).count() >= failures
            && reclaims.last().is_some_and(|(_, _, ok)| *ok),
        "seed={seed}: expected at least {failures} failing reclaims of {pred} then one that \
         succeeds; got {reclaims:?}"
    );
    assert_reclaim_cadence(seed, &leftover, &fixture.journal);
    let released = fixture.journal.lease_events(LEASE_RELEASED, &pred);
    assert!(
        released.len() == 1 && released[0] > disarmed_at && released[0] < successor_at,
        "seed={seed}: {pred} is released once, after the disarm (journal index {disarmed_at}) and \
         before its successor's admission (journal index {successor_at}); got {released:?}"
    );
    // The held population at every admission never exceeds the cap.
    let mut held = 0_usize;
    let mut peak = 0_usize;
    for entry in fixture.journal.entries() {
        match entry {
            Entry::Provision { .. } => {
                held += 1;
                peak = peak.max(held);
            }
            Entry::Lease { name, .. } if name == LEASE_RELEASED => held = held.saturating_sub(1),
            _ => {}
        }
    }
    assert!(
        peak <= cap,
        "seed={seed}: the held population peaked at {peak} > cap {cap} (provisions minus \
         {LEASE_RELEASED} events)"
    );
    eprintln!("seed={seed} S-ND295-57 at-cap: GREEN");
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-57 — Leftover networks are reclaimed until released, across stopped and deleted workloads.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER step 07-03 (S-ND295-57)"]
async fn leftover_networks_are_reclaimed_until_released_on_every_path() {
    for seed in seeds() {
        eprintln!("seed={seed} body=leftover_networks_are_reclaimed_until_released_on_every_path");
        leftover_networks_on_every_path(seed).await;
    }
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-57 — Leftover networks are reclaimed until released, across stopped and deleted workloads.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER step 07-03 (S-ND295-57)"]
async fn a_not_yet_due_restart_keeps_its_predecessor_at_the_cap() {
    for seed in seeds() {
        eprintln!("seed={seed} body=a_not_yet_due_restart_keeps_its_predecessor_at_the_cap");
        not_yet_due_restart_at_the_cap(seed).await;
    }
}
