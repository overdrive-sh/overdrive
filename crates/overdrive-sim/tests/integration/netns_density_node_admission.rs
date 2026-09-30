//! netns-density-295 S-ND295-05D — across a whole node, held guest-network
//! attachments never exceed the cap (seeded Tier-1 simulation; the recovery
//! proof §3.2, moved here and re-targeted to the accepted DESIGN).
//!
//! # Contract under test
//!
//! D-295-R6, R7, R8, R11 (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the cap, the pool, placement, and restart gating); ADR-0132 to ADR-0134):
//! a node holds at most `MAX_GUEST_NETWORK_ATTACHMENTS` = 16,384 guest-network
//! leases. A lease is held from the pool's `assign` until the allocation's
//! cleanup has finished and the pool releases it. The held population is
//! therefore Admitted plus Retiring leases: a Retiring lease counts (R6/R7),
//! so the former "not decided" framing of a retiring attachment is retired.
//! Placement returns `NoCapacity` while `held >= cap`, reading occupancy
//! through the production `GuestAttachmentView` over the server's pool. At
//! the cap a due crash restart reclaims its predecessor's lease
//! (`ReclaimAllocationNetwork`) before the successor is admitted; below the
//! cap the successor overlaps its predecessor (ADR-0106).
//!
//! Lease classes, observed only at the guest-network driven port, through the
//! pinned lease events, and in the observation store (never at private
//! state):
//!
//! | Class                 | Lease held | Allocation row                               | Counts |
//! |-----------------------|------------|----------------------------------------------|--------|
//! | in flight             | yes        | none / `Pending` / `Draining` / `Suspended`  | yes    |
//! | Running               | yes        | `Running`                                    | yes    |
//! | terminal predecessor  | yes        | `Failed` / `Terminated` (Retiring once its cleanup began) | yes |
//! | released              | no         | any                                          | no     |
//!
//! # Invariants asserted
//!
//! | Id          | Kind     | Statement |
//! |-------------|----------|-----------|
//! | NA-1        | safety   | A new workload placed while 16,384 leases are held acquires no attachment, both with 16,384 Running and with 16,383 Running plus one crashed predecessor whose lease is still held. |
//! | NA-2        | safety   | A new workload placed while 16,383 are Running and one admitted workload is in flight (lease held, no `Running` row yet) acquires no attachment. |
//! | NA-4a       | safety   | Replacing a crashed Service never makes the held population exceed 16,384 at any lease acquisition. |
//! | NA-4a-L     | liveness | Once room exists, the crashed Service is replaced and its old lease released (a guard against NA-4a passing vacuously). |
//! | NA-4b       | safety   | Resuming an operator-stopped Service never makes the held population exceed 16,384 at any lease acquisition. |
//! | NA-4b-L     | liveness | Once room exists, the stopped Service resumes (a guard against NA-4b passing vacuously). |
//! | NA-5        | liveness | With 16,383 leases held after a release, a new workload's placement acquires an attachment (in-flight, Service, and plain placement variants). |
//! | NA-RECREATE | safety   | At the cap, the crashed predecessor's `guest_network.lease_released` precedes its successor's `provision`. |
//! | NA-OVERLAP  | safety   | The peak held population, retiring predecessors and their replacements included, never exceeds 16,384 (formerly the observation OBS-OVERLAP). |
//! | NA-G        | safety   | The peak held population at any lease acquisition over the whole run is at most 16,384. |
//! | NA-VIEW     | safety   | Each at-cap placement window (NA-1 twice, NA-2) holds no `guest_network.admission_refused`: placement over the production `GuestAttachmentView` returns `NoCapacity` before any dispatch, so the pool's own refusal is never what stops the overflow. A refusal there means the view under-reported occupancy (DR-12). |
//! | NA-E7       | safety   | At most one refused dispatch per contended slot (E7): every allocation, and every workload, sees at most one `guest_network.admission_refused` over the run. Each workload in this schedule contends for at most one slot, and a raced restart refusal advances the successor id, so the count is taken per workload as well as per allocation. |
//! | NA-E7-C     | safety   | One contended slot (the `Contended` block): two new workloads are placed on the one free slot before either dispatches; exactly one wins it, the other's dispatch is refused exactly once (`guest_network.admission_refused` naming its allocation — the witness that NA-E7 is not vacuous), and its immediate re-evaluation is refused by placement with no second refused dispatch. |
//!
//! # Production owner path (causes are driven, consequences are observed)
//!
//! - **Operator input**: the Job and Service intents, stop sentinel, and
//!   restart generation reach the real `LocalIntentStore` with exactly the
//!   store effects of `handlers::submit_workload` (`put_if_absent` of
//!   `workloads/<id>`, then `put` of `workloads/<id>/kind`; for a Service also
//!   the frontend-address assignment, VIP allocation, and listener-fact
//!   upsert), `handlers::stop_workload` (`put_if_absent` of
//!   `workloads/<id>/stop`), and `handlers::restart_workload` (`txn[IncrementU64
//!   generation, Delete stop]`). The handlers take axum extractors and
//!   `overdrive-sim` has no axum edge, so their store effects are reproduced
//!   here, not the HTTP framing. The handlers author no admission decision.
//! - **Every admission decision** runs through
//!   `run_convergence_tick_with_guest_network_provisioner_for_test`, the
//!   production evaluation with only the guest-network driven port supplied by
//!   the test (the accepted C-295-B seam; it reads the EXEC gate and the pool
//!   from `state`, FD § "C-295-B — network provisioner boundary" (the B-1 pin that both test helpers read the gate and the pool from `state`)). The registered `WorkloadLifecycle` hydrates
//!   `desired` and `actual`, including the guest-attachment occupancy. The pure
//!   `reconcile` calls `overdrive_core::scheduler::schedule` or emits the
//!   restart or reclaim action, the runtime `ViewStore` persists the view, and
//!   the production action shim dispatches. The shim takes the lease from the
//!   server's own pool — one pool per server (R6), never a process global —
//!   and then calls the port.
//! - **Seam fixture** (TS § *Seam fixture*): [`SimNode::compose`] is this
//!   file's one helper. It holds one owner instance, [`LeaseLedger`], which
//!   implements both `GuestNetworkProvisioner` and `SharedGuestNetworkOwner`
//!   by delegation to `SimSharedGuestNetworkOwner`; one
//!   `GuestNetworkExecWiring` over the fixture clock, whose gate is opened with
//!   `open_after_boot()` before the first dispatch; one `GuestAddressPool` from
//!   the doc-hidden `GuestAddressPool::new` with today's pool constants; and
//!   one started `MtlsInterceptWorker` over `SimMtlsEnforcement`,
//!   `SimMtlsResolve`, and `SimMtlsIntercept`. The ledger is the seams'
//!   `provisioner` and the owner `AppState` receives, so every owner
//!   observation has one source. `AppState` constructors are unchanged until
//!   DELIVER 05-01 (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the `AppState` constructors)); until then the helper passes today's inputs,
//!   and 05-01 changes that one call to pass the worker, owner, gate, and pool.
//! - **Driven ports substituted**: `SimDriver`, `SimObservationStore`,
//!   `SimViewStore`, `SimClock`, `SimCa`, `SimDataplane`, the three mTLS sim
//!   ports above, and `SimSharedGuestNetworkOwner` behind the ledger. On
//!   request the ledger parks exactly one `provision` call, after the pool
//!   lease was taken. That produces the in-flight interleaving that
//!   `spawn_convergence_loop` reaches by running up to
//!   `CONVERGENCE_MAX_IN_FLIGHT = 8` evaluations on distinct targets at once.
//!   The decorator controls ordering only. It never authors a row, a lease, a
//!   gate transition, or a decision.
//! - **Contended slot** (NA-E7-C): the runtime's `ViewStore` is the fixture's
//!   `SimViewStore` behind [`OrderingViewStore`], a test-local decorator that,
//!   on request, parks one evaluation's view write-through. The tick persists
//!   the next view after `reconcile` has placed the workload and before the
//!   action shim dispatches (ADR-0035 §5 step 7 before step 9), and a fresh
//!   placement always changes the view (it reserves the allocation id), so
//!   the park holds evaluation A between its placement and its dispatch.
//!   Evaluation B of another workload then runs whole on the same free slot.
//!   Production reaches the same interleaving: `spawn_convergence_loop` runs
//!   up to eight evaluations on distinct targets at once, and each
//!   write-through is a real fsync await. Like the ledger, the decorator
//!   controls ordering only; every write it parks is delegated unchanged.
//! - **Crash stimulus**: `SimDriver::inject_exit_after(.., Crashed)`, consumed
//!   by the production `worker::exit_observer`, which authors the `Failed`
//!   row. No row is seeded. The crash victim is a Service because a crashed
//!   Job is a natural exit: `WorkloadLifecycle` finalizes it and releases its
//!   attachment, but never replaces it.
//!
//! # Outcome oracle
//!
//! The pool's own counts are crate-private (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the crate-private pool operations)), so the held
//! population is counted from outside: every `provision` the ledger sees adds
//! a held lease, and every `guest_network.lease_released { alloc }` event
//! removes one (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the admission refusal projection and the lease events)). A test-local tracing layer ([`LeaseEventLayer`])
//! captures `lease_released`, `lease_retired`, and `admission_refused` and
//! appends them, with each `provision`, to one ordered journal. The held
//! count at each acquisition is the NA-1..NA-G measure; the journal order is
//! the NA-RECREATE oracle; the `admission_refused` entries inside each
//! placement window are the NA-VIEW oracle, and their counts over the run the
//! NA-E7 oracle. Each placement report also carries the census (Running, in
//! flight, terminal predecessor) read from the observation store.
//!
//! # Evidence tier, bounds, and reproduction
//!
//! Tier 1: in-process, current-thread runtime, explicit interleaving, seeded
//! victims, fill order, and block order. The run is bounded by the fill
//! (16,384 evaluations), four blocks, and fixed settle budgets, and creates no
//! kernel object. No wall time is read: the only time source is the fixture's
//! `SimClock`, advanced by the harness, and every wait yields to the runtime.
//! Filling one node to 16,384 is quadratic in the cap (about 190 s per seed),
//! which is why this body sits in the integration binary with a nextest
//! timeout override for this test alone. Until the DELIVER step that
//! carries B-7 (05-01 at the latest), `SimMtlsIntercept`'s bind still opens a
//! plain loopback listener for the worker's two shared legs; after it the
//! worker binds nothing (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the effect on D-295-R16, E16, and lane classification)). The worker's shared-leg tasks end when
//! the last worker reference drops.
//!
//! The seeds come from `OVERDRIVE_ND295_ADMISSION_SEED`, a comma-separated
//! list of `u64` (defaults `186055177052160001` = `0x0295_0032_A0D1_0001` and
//! `295032`, the two seeds the recovery run recorded). The body runs each seed
//! on a fresh node, in order, and prints the seed with every verdict. Reproduce
//! one seed with
//! `OVERDRIVE_ND295_ADMISSION_SEED=<seed> cargo xtask lima run -- cargo nextest run -p overdrive-sim --test integration --features integration-tests --run-ignored all --no-capture -E 'test(node_wide_attachment_admission_never_exceeds_the_t1_cap_across_workloads)'`.

#![allow(
    clippy::expect_used,
    clippy::print_stderr,
    clippy::too_many_lines,
    clippy::doc_markdown,
    reason = "seeded proof harness: fixture preconditions fail loudly, the verdict table is the evidence"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bytes::Bytes;
use overdrive_control_plane::guest_network::{
    GuestAddressPool, GuestNetworkPlan, GuestNetworkProvisioner, Result as GuestNetworkResult,
    SharedGuestNetworkAudit, SharedGuestNetworkAuditError, SharedGuestNetworkOwner, TapActivation,
    TapQuiescence,
};
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, run_convergence_tick_with_guest_network_provisioner_for_test,
};
use overdrive_control_plane::view_store::{ProbeError, Result as ViewStoreResult, ViewStore};
use overdrive_control_plane::worker::exit_observer;
use overdrive_control_plane::{AppState, workload_lifecycle};
use overdrive_core::aggregate::{
    DriverInput, IntentKey, Job, JobSpecInput, ResourcesInput, Service, VmInput, WorkloadIntent,
    WorkloadKind,
};
use overdrive_core::api::{ListenerInput, ServiceSpecInput};
use overdrive_core::guest_network::{
    GuestNetworkExecGate, GuestNetworkExecSupervisor, GuestNetworkExecWiring,
    MAX_GUEST_NETWORK_ATTACHMENTS,
};
use overdrive_core::id::{AllocationId, MeshServiceName, NodeId, WorkloadId};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{DriverType, ExitKind};
use overdrive_core::traits::intent_store::{IntentStore, PutOutcome, TxnOp};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
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
use tokio::sync::oneshot;
use tracing::field::{Field, Visit};
use tracing::subscriber::Interest;
use tracing::{Event, Metadata, Subscriber};
use tracing_subscriber::Registry;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

/// The accepted fixed node-wide cap (D-295-R6; `MAX_GUEST_NETWORK_ATTACHMENTS`
/// in `overdrive_core::guest_network`, FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the `MAX_GUEST_NETWORK_ATTACHMENTS` constant)).
const CAP: usize = 16_384;
const _: () = assert!(
    MAX_GUEST_NETWORK_ATTACHMENTS as usize == CAP,
    "S-ND295-05D asserts the accepted 16,384 cap"
);
/// Today's guest-address pool composition. R6 composes the per-server pool
/// with the same node prefix, bridge, gateway, and DNS address the shared
/// owner is composed with (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (one pool per server: the doc-hidden `GuestAddressPool::new`)).
const POOL_PREFIX: &str = "100.95.0.0/16";
const POOL_BRIDGE: &str = "ovd-gbr0";
const POOL_GATEWAY: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
const POOL_DNS: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
/// The Phase-1 single-node baseline id the scheduler places onto
/// (`workload_lifecycle::baseline_nodes_phase1`).
const NODE: &str = "local";
/// Per-workload memory demand; see `SimNode::operator_deploys`.
const DEMAND_MEMORY_BYTES: u64 = 256 * 1024;
const SEED_ENV: &str = "OVERDRIVE_ND295_ADMISSION_SEED";
/// `186055177052160001` and `295032`, the two seeds the recovery run recorded
/// (TS S-ND295-05D).
const DEFAULT_SEEDS: [u64; 2] = [0x0295_0032_A0D1_0001, 295_032];
/// Bounded convergence retries for release, replacement, and resume settling.
const SETTLE_TICKS: usize = 6;
/// Bounded clock nudges while waiting for the exit observer.
const CRASH_POLL_BUDGET: usize = 2_000;
/// The pinned lease events (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the admission refusal projection and the lease events)).
const LEASE_RETIRED: &str = "guest_network.lease_retired";
const LEASE_RELEASED: &str = "guest_network.lease_released";
const ADMISSION_REFUSED: &str = "guest_network.admission_refused";
const LEASE_EVENTS: [&str; 3] = [LEASE_RETIRED, LEASE_RELEASED, ADMISSION_REFUSED];

/// The seeds to run, in order: `OVERDRIVE_ND295_ADMISSION_SEED` as a
/// comma-separated `u64` list, or [`DEFAULT_SEEDS`].
fn seeds() -> Vec<u64> {
    std::env::var(SEED_ENV).ok().map_or_else(
        || DEFAULT_SEEDS.to_vec(),
        |raw| {
            raw.split(',')
                .map(|seed| {
                    seed.trim().parse().unwrap_or_else(|_| {
                        panic!("{SEED_ENV} must be a comma-separated list of u64 seeds: {raw}")
                    })
                })
                .collect()
        },
    )
}

// ---------------------------------------------------------------------------
// Lease observation: the owner port, decorated, plus the pinned lease events.
// ---------------------------------------------------------------------------

/// One entry of the ordered lease journal.
#[derive(Debug, Clone)]
enum LeaseEvent {
    /// `provision` reached the port after the pool's `assign`; `held` is the
    /// held population including this lease, `overlap` whether another lease
    /// of the same workload (its predecessor) was still held.
    Provision { alloc: AllocationId, held: usize, overlap: bool },
    /// `guest_network.lease_retired { alloc }`.
    Retired { alloc: AllocationId },
    /// `guest_network.lease_released { alloc }`; `was_held` is false for a
    /// release of a lease the ledger never saw provisioned.
    Released { alloc: AllocationId, was_held: bool },
    /// `guest_network.admission_refused { alloc, held, retiring, cap }`.
    Refused { alloc: Option<AllocationId>, fields: String },
    /// A lease event whose `alloc` field did not parse.
    Malformed { name: &'static str, fields: String },
}

impl std::fmt::Display for LeaseEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provision { alloc, held, overlap } => {
                write!(f, "provision {alloc} (held {held}, predecessor still held={overlap})")
            }
            Self::Retired { alloc } => write!(f, "{LEASE_RETIRED} {alloc}"),
            Self::Released { alloc, was_held } => {
                write!(f, "{LEASE_RELEASED} {alloc} (was held={was_held})")
            }
            Self::Refused { alloc, fields } => {
                write!(f, "{ADMISSION_REFUSED} {alloc:?} [{fields}]")
            }
            Self::Malformed { name, fields } => write!(f, "malformed {name} [{fields}]"),
        }
    }
}

fn render_events(events: &[LeaseEvent]) -> String {
    events.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
}

/// The field values of one captured event, rendered as text.
#[derive(Debug, Default)]
struct LeaseEventFields(BTreeMap<&'static str, String>);

impl LeaseEventFields {
    fn render(&self) -> String {
        self.0.iter().map(|(name, value)| format!("{name}={value}")).collect::<Vec<_>>().join(" ")
    }
}

impl Visit for LeaseEventFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name(), value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name(), format!("{value:?}"));
    }
}

/// Parse the `alloc` field, whether it was recorded with `%` or `?`.
fn parse_alloc(raw: &str) -> Option<AllocationId> {
    if let Ok(alloc) = AllocationId::new(raw) {
        return Some(alloc);
    }
    let start = raw.find('"')? + 1;
    let end = raw.rfind('"')?;
    raw.get(start..end).and_then(|inner| AllocationId::new(inner).ok())
}

#[derive(Default)]
struct LedgerState {
    held: BTreeSet<AllocationId>,
    held_per_workload: BTreeMap<String, usize>,
    retired: BTreeSet<AllocationId>,
    journal: Vec<LeaseEvent>,
    measuring: bool,
    held_peak: usize,
    held_peak_at: Option<AllocationId>,
    lease_peak: usize,
    overlap_peak: Option<(usize, AllocationId)>,
    window_peak: Option<usize>,
    window_acquired: Vec<AllocationId>,
}

impl LedgerState {
    fn acquire(&mut self, alloc: &AllocationId) {
        if self.held.insert(alloc.clone())
            && let Some(workload) = workload_of(alloc)
        {
            *self.held_per_workload.entry(workload.to_owned()).or_default() += 1;
        }
        let held = self.held.len();
        let overlap = workload_of(alloc)
            .is_some_and(|workload| self.held_per_workload.get(workload).is_some_and(|n| *n > 1));
        self.lease_peak = self.lease_peak.max(held);
        if overlap && self.overlap_peak.as_ref().is_none_or(|(peak, _)| held > *peak) {
            self.overlap_peak = Some((held, alloc.clone()));
        }
        if self.measuring && held > self.held_peak {
            self.held_peak = held;
            self.held_peak_at = Some(alloc.clone());
        }
        self.window_peak = Some(self.window_peak.map_or(held, |peak| peak.max(held)));
        self.window_acquired.push(alloc.clone());
        self.journal.push(LeaseEvent::Provision { alloc: alloc.clone(), held, overlap });
    }

    fn release(&mut self, alloc: AllocationId) {
        let was_held = self.held.remove(&alloc);
        if was_held && let Some(workload) = workload_of(&alloc) {
            let emptied = self.held_per_workload.get_mut(workload).is_some_and(|count| {
                *count = count.saturating_sub(1);
                *count == 0
            });
            if emptied {
                self.held_per_workload.remove(workload);
            }
        }
        self.retired.remove(&alloc);
        self.journal.push(LeaseEvent::Released { alloc, was_held });
    }

    fn record_lease_event(&mut self, name: &'static str, fields: &LeaseEventFields) {
        let alloc = fields.0.get("alloc").and_then(|raw| parse_alloc(raw));
        match (name, alloc) {
            (LEASE_RELEASED, Some(alloc)) => self.release(alloc),
            (LEASE_RETIRED, Some(alloc)) => {
                self.retired.insert(alloc.clone());
                self.journal.push(LeaseEvent::Retired { alloc });
            }
            (ADMISSION_REFUSED, alloc) => {
                self.journal.push(LeaseEvent::Refused { alloc, fields: fields.render() });
            }
            (_, _) => self.journal.push(LeaseEvent::Malformed { name, fields: fields.render() }),
        }
    }
}

/// Captures the pinned lease events into the ledger's journal. Installed as
/// the thread-local default subscriber for the test body; the current-thread
/// runtime polls every task that dispatches an action on that thread.
struct LeaseEventLayer {
    state: Arc<Mutex<LedgerState>>,
}

fn is_lease_event(metadata: &Metadata<'_>) -> bool {
    metadata.is_event() && LEASE_EVENTS.contains(&metadata.name())
}

impl<S> Layer<S> for LeaseEventLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if is_lease_event(metadata) { Interest::always() } else { Interest::never() }
    }

    fn enabled(&self, metadata: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        is_lease_event(metadata)
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut fields = LeaseEventFields::default();
        event.record(&mut fields);
        self.state.lock().record_lease_event(event.metadata().name(), &fields);
    }
}

struct Park {
    entered: oneshot::Sender<AllocationId>,
    release: oneshot::Receiver<()>,
}

/// The fixture's one owner instance: records lease acquisition at the
/// guest-network driven port and delegates every owner operation to
/// `SimSharedGuestNetworkOwner`.
///
/// The action shim assigns the pool lease and immediately calls
/// `GuestNetworkProvisioner::provision`, so a `provision` call is a lease
/// acquisition. A lease is released only when the pool emits
/// `guest_network.lease_released`, which [`LeaseEventLayer`] records into the
/// same state. The held set is therefore exactly the node's held leases,
/// Admitted and Retiring alike.
struct LeaseLedger {
    inner: SimSharedGuestNetworkOwner,
    state: Arc<Mutex<LedgerState>>,
    park_next: Mutex<Option<Park>>,
}

impl LeaseLedger {
    fn new() -> Self {
        Self {
            inner: SimSharedGuestNetworkOwner::default(),
            state: Arc::new(Mutex::new(LedgerState::default())),
            park_next: Mutex::new(None),
        }
    }

    fn event_layer(&self) -> LeaseEventLayer {
        LeaseEventLayer { state: Arc::clone(&self.state) }
    }

    fn held(&self) -> BTreeSet<AllocationId> {
        self.state.lock().held.clone()
    }

    fn held_count(&self) -> usize {
        self.state.lock().held.len()
    }

    fn retired_count(&self) -> usize {
        self.state.lock().retired.len()
    }

    fn holds(&self, alloc: &AllocationId) -> bool {
        self.state.lock().held.contains(alloc)
    }

    fn holds_for(&self, workload: &str) -> bool {
        self.state.lock().held_per_workload.contains_key(workload)
    }

    /// After the fill: track the held peak at every lease acquisition. The
    /// fill itself is bounded by construction (the harness checks one new
    /// lease per filler and no release).
    fn start_measuring(&self) {
        let mut state = self.state.lock();
        state.held_peak = state.held.len();
        state.held_peak_at = None;
        state.measuring = true;
    }

    fn begin_window(&self) {
        let mut state = self.state.lock();
        state.window_peak = None;
        state.window_acquired.clear();
    }

    fn window(&self) -> (Option<usize>, Vec<AllocationId>) {
        let state = self.state.lock();
        (state.window_peak, state.window_acquired.clone())
    }

    fn journal_len(&self) -> usize {
        self.state.lock().journal.len()
    }

    fn journal_since(&self, start: usize) -> Vec<LeaseEvent> {
        self.state.lock().journal.get(start..).map(<[LeaseEvent]>::to_vec).unwrap_or_default()
    }

    /// `(held peak after the fill, where, lease peak over the run, overlap peak)`.
    fn peaks(&self) -> (usize, Option<AllocationId>, usize, Option<(usize, AllocationId)>) {
        let state = self.state.lock();
        (state.held_peak, state.held_peak_at.clone(), state.lease_peak, state.overlap_peak.clone())
    }

    /// Journal entries the harness cannot account for: malformed lease events,
    /// releases of leases the ledger never saw provisioned, and refusals that
    /// name no parseable allocation (NA-E7 could not attribute them).
    fn unaccounted(&self) -> Vec<LeaseEvent> {
        self.state
            .lock()
            .journal
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    LeaseEvent::Malformed { .. }
                        | LeaseEvent::Released { was_held: false, .. }
                        | LeaseEvent::Refused { alloc: None, .. }
                )
            })
            .cloned()
            .collect()
    }

    /// Park the next `provision` call after its lease was taken. Returns the
    /// signal that it parked and the handle that releases it.
    fn arm_park(&self) -> (oneshot::Receiver<AllocationId>, oneshot::Sender<()>) {
        let (entered, entered_rx) = oneshot::channel();
        let (release_tx, release) = oneshot::channel();
        *self.park_next.lock() = Some(Park { entered, release });
        (entered_rx, release_tx)
    }
}

#[async_trait::async_trait]
impl GuestNetworkProvisioner for LeaseLedger {
    async fn provision(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        self.state.lock().acquire(plan.alloc());
        let park = self.park_next.lock().take();
        if let Some(park) = park {
            park.entered.send(plan.alloc().clone()).expect("harness is awaiting the park signal");
            park.release.await.expect("harness releases the parked provision");
        }
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
impl SharedGuestNetworkOwner for LeaseLedger {
    async fn probe_startup(&self) -> GuestNetworkResult<()> {
        self.inner.probe_startup().await
    }

    async fn sweep_stale(&self) -> GuestNetworkResult<()> {
        self.inner.sweep_stale().await
    }

    async fn converge_shared(&self) -> GuestNetworkResult<()> {
        self.inner.converge_shared().await
    }

    async fn audit_shared(
        &self,
    ) -> std::result::Result<SharedGuestNetworkAudit, SharedGuestNetworkAuditError> {
        self.inner.audit_shared().await
    }

    async fn quiesce_managed_taps(&self) -> GuestNetworkResult<TapQuiescence> {
        self.inner.quiesce_managed_taps().await
    }

    async fn restore_quiesced_taps(&self) -> GuestNetworkResult<()> {
        self.inner.restore_quiesced_taps().await
    }
}

/// `mint_alloc_id` grammar: `alloc-<workload>-<attempt>`.
fn workload_of(alloc: &AllocationId) -> Option<&str> {
    alloc.as_str().strip_prefix("alloc-")?.rsplit_once('-').map(|(workload, _)| workload)
}

/// The `guest_network.admission_refused` entries of `events`.
fn refusals(events: &[LeaseEvent]) -> Vec<LeaseEvent> {
    events.iter().filter(|event| matches!(event, LeaseEvent::Refused { .. })).cloned().collect()
}

/// NA-E7 counts: refusals per allocation and per workload. A refusal that
/// names no allocation is a harness failure caught by
/// [`LeaseLedger::unaccounted`] before the verdicts, so it is not counted here.
fn refusal_counts(
    events: &[LeaseEvent],
) -> (BTreeMap<AllocationId, usize>, BTreeMap<String, usize>) {
    let mut per_alloc: BTreeMap<AllocationId, usize> = BTreeMap::new();
    let mut per_workload: BTreeMap<String, usize> = BTreeMap::new();
    for event in events {
        if let LeaseEvent::Refused { alloc: Some(alloc), .. } = event {
            *per_alloc.entry(alloc.clone()).or_default() += 1;
            let workload = workload_of(alloc).unwrap_or_else(|| alloc.as_str()).to_owned();
            *per_workload.entry(workload).or_default() += 1;
        }
    }
    (per_alloc, per_workload)
}

/// NA-RECREATE over the journal of one replacement: the predecessor's release
/// precedes the successor's first `provision`.
fn recreate_order(
    events: &[LeaseEvent],
    workload: &str,
    predecessor: &AllocationId,
) -> (bool, String) {
    let released = events.iter().position(
        |event| matches!(event, LeaseEvent::Released { alloc, .. } if alloc == predecessor),
    );
    let successor = events.iter().position(|event| match event {
        LeaseEvent::Provision { alloc, .. } => {
            let of_workload = workload_of(alloc) == Some(workload);
            of_workload && alloc != predecessor
        }
        LeaseEvent::Retired { .. }
        | LeaseEvent::Released { .. }
        | LeaseEvent::Refused { .. }
        | LeaseEvent::Malformed { .. } => false,
    });
    let relevant: Vec<LeaseEvent> = events
        .iter()
        .filter(|event| match event {
            LeaseEvent::Provision { alloc, .. }
            | LeaseEvent::Retired { alloc }
            | LeaseEvent::Released { alloc, .. } => workload_of(alloc) == Some(workload),
            LeaseEvent::Refused { .. } | LeaseEvent::Malformed { .. } => true,
        })
        .cloned()
        .collect();
    let rendered = render_events(&relevant);
    match (released, successor) {
        (Some(released), Some(provision)) => (
            released < provision,
            format!(
                "predecessor {predecessor} released at journal entry {released}, successor \
                 provisioned at entry {provision}; {workload} lease journal: {rendered}"
            ),
        ),
        (released, None) => (
            false,
            format!(
                "no successor provision observed (vacuous; see NA-4a-L); predecessor release \
                 entry={released:?}; {workload} lease journal: {rendered}"
            ),
        ),
        (None, Some(provision)) => (
            false,
            format!(
                "successor provisioned at journal entry {provision} while predecessor \
                 {predecessor} was never released; {workload} lease journal: {rendered}"
            ),
        ),
    }
}

// ---------------------------------------------------------------------------
// Node composition (the seam fixture: production AppState + registered
// WorkloadLifecycle + the four inputs DELIVER 05-01 passes to AppState).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Census {
    running: usize,
    in_flight: usize,
    /// Held leases whose allocation row is terminal: a crashed or stopped
    /// predecessor whose cleanup has not finished (Retiring once it began).
    terminal: usize,
}

impl Census {
    const fn held(self) -> usize {
        self.running + self.in_flight + self.terminal
    }
}

#[derive(Debug, Clone)]
struct Decision {
    before: Census,
    retired_before: usize,
    node_running_rows: usize,
    admitted: bool,
    acquired: Vec<AllocationId>,
    held_at_acquisition: Option<usize>,
    /// The `guest_network.admission_refused` entries journaled during this
    /// placement's evaluation (the NA-VIEW oracle).
    refused: Vec<LeaseEvent>,
    dispatch: Result<(), String>,
}

impl Decision {
    fn evidence(&self) -> String {
        format!(
            "before: held={} (Running {}, in flight {}, terminal predecessor {}; {} retired by \
             event); node-wide Running rows={}; acquired attachment={} {:?}; held population at \
             that acquisition={:?}; {ADMISSION_REFUSED} in the window=[{}]; dispatch={:?}",
            self.before.held(),
            self.before.running,
            self.before.in_flight,
            self.before.terminal,
            self.retired_before,
            self.node_running_rows,
            self.admitted,
            self.acquired,
            self.held_at_acquisition,
            render_events(&self.refused),
            self.dispatch,
        )
    }
}

/// The inputs DELIVER 05-01's `AppState` constructors take beside the owner
/// (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the `AppState` constructors)). The seam fixture holds them from phase B on so that
/// constructor change is one line.
#[allow(
    dead_code,
    reason = "passed to AppState::new at DELIVER 05-01 (TS § Seam fixture); held until then"
)]
struct HeldForAppState {
    mtls_worker: Arc<MtlsInterceptWorker>,
    exec_gate: Arc<GuestNetworkExecGate>,
    exec_supervisor: Arc<GuestNetworkExecSupervisor>,
    guest_pool: Arc<GuestAddressPool>,
}

/// One parked view write-through: the evaluation's target, the signal that
/// it parked, and the handle that releases it.
struct ParkedWrite {
    target: TargetResource,
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

/// Test-local ordering decorator over the fixture's `SimViewStore` (the NA-E7-C
/// contended-slot stimulus). On request it parks the next write-through for
/// one target — the tick's step between `reconcile` and dispatch — until the
/// harness releases it, then delegates the write unchanged. Every other call
/// delegates at once. It authors no view, row, lease, or decision.
struct OrderingViewStore {
    inner: SimViewStore,
    park_next: Mutex<Option<ParkedWrite>>,
}

impl OrderingViewStore {
    fn new() -> Self {
        Self { inner: SimViewStore::new(), park_next: Mutex::new(None) }
    }

    /// Park the next write-through for `target`; returns the signal that it
    /// parked and the handle that releases it.
    fn arm_park(&self, target: TargetResource) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (entered, entered_rx) = oneshot::channel();
        let (release_tx, release) = oneshot::channel();
        let previous = self.park_next.lock().replace(ParkedWrite { target, entered, release });
        assert!(previous.is_none(), "one view write-through park at a time");
        (entered_rx, release_tx)
    }
}

#[async_trait::async_trait]
impl ViewStore for OrderingViewStore {
    async fn bulk_load_bytes(
        &self,
        reconciler: &'static str,
    ) -> ViewStoreResult<BTreeMap<TargetResource, Vec<u8>>> {
        self.inner.bulk_load_bytes(reconciler).await
    }

    async fn write_through_bytes(
        &self,
        reconciler: &'static str,
        target: &TargetResource,
        cbor: &[u8],
    ) -> ViewStoreResult<()> {
        let parked = {
            let mut slot = self.park_next.lock();
            if slot.as_ref().is_some_and(|park| &park.target == target) {
                slot.take()
            } else {
                None
            }
        };
        if let Some(park) = parked {
            park.entered.send(()).expect("the harness is awaiting the view-park signal");
            park.release.await.expect("the harness releases the parked view write-through");
        }
        self.inner.write_through_bytes(reconciler, target, cbor).await
    }

    async fn delete(
        &self,
        reconciler: &'static str,
        target: &TargetResource,
    ) -> ViewStoreResult<()> {
        self.inner.delete(reconciler, target).await
    }

    async fn probe(&self) -> std::result::Result<(), ProbeError> {
        self.inner.probe().await
    }
}

struct SimNode {
    _tmp: TempDir,
    state: AppState,
    /// The runtime's `ViewStore`: the ordering decorator over `SimViewStore`.
    views: Arc<OrderingViewStore>,
    driver: Arc<SimDriver>,
    clock: Arc<SimClock>,
    /// The one owner instance: the seams' `provisioner` and `AppState`'s owner.
    owner: Arc<LeaseLedger>,
    _held_for_app_state: HeldForAppState,
    reconciler: ReconcilerName,
    tick: AtomicU64,
    seed: u64,
    trace: Mutex<Vec<String>>,
}

impl SimNode {
    /// This file's seam fixture helper (TS § *Seam fixture*).
    async fn compose(seed: u64, owner: Arc<LeaseLedger>) -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let views = Arc::new(OrderingViewStore::new());
        let mut runtime =
            ReconcilerRuntime::new(tmp.path(), Arc::clone(&views) as Arc<dyn ViewStore>)
                .expect("reconciler runtime");
        runtime.register(workload_lifecycle()).await.expect("register workload-lifecycle");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("intent store"));
        let node_id = NodeId::new(NODE).expect("node id");
        let obs: Arc<dyn ObservationStore> =
            Arc::new(SimObservationStore::single_peer(node_id.clone(), seed));
        let clock = Arc::new(SimClock::new());
        let driver = Arc::new(SimDriver::with_clock(DriverType::Vm, clock.clone()));
        let allocator =
            overdrive_control_plane::test_default_allocator(store.clone() as Arc<dyn IntentStore>);

        // One started worker over the three mTLS sim ports (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (`AppState` constructors: why `mtls_worker` is a parameter)).
        let identity: Arc<dyn IdentityRead> = Arc::new(SimIdentityRead::new(BTreeMap::new(), None));
        let enforcement: Arc<dyn MtlsEnforcement> =
            Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()));
        let resolve: Arc<dyn MtlsResolve> =
            Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh));
        let mtls_worker = Arc::new(MtlsInterceptWorker::new(
            enforcement,
            resolve,
            clock.clone(),
            Arc::new(SimMtlsIntercept::new()),
        ));
        mtls_worker.start_shared_owner().await.unwrap_or_else(|error| {
            panic!(
                "seed={seed}: harness precondition failed (not a contract verdict): the worker's \
                 shared owner did not start: {error}"
            )
        });

        // One EXEC wiring over the fixture clock. Only its paired supervisor
        // moves the gate; it is opened before the first dispatch
        // (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (gate state outside `run_server*`)).
        let exec = GuestNetworkExecWiring::new(clock.clone());
        let exec_gate = exec.gate();
        let exec_supervisor = exec.supervisor();
        assert!(
            exec_supervisor.open_after_boot(),
            "seed={seed}: harness precondition failed (not a contract verdict): the fixture's \
             EXEC gate did not open from BootClosed"
        );

        // One pool per server, composed with today's constants (R6).
        let guest_pool = Arc::new(GuestAddressPool::new(
            POOL_PREFIX.parse().expect("pool node prefix"),
            POOL_BRIDGE.to_owned(),
            POOL_GATEWAY,
            POOL_DNS,
        ));

        // DELIVER 05-01 changes this call and no other line of the file:
        // `AppState::new` appends `mtls_worker`, `shared_guest_network`,
        // `guest_network_exec`, and `guest_pool` (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the `AppState` constructors)), passed here as
        // `Arc::clone(&mtls_worker)`,
        // `Arc::clone(&owner) as Arc<dyn SharedGuestNetworkOwner>`,
        // `Arc::clone(&exec_gate)`, and `Arc::clone(&guest_pool)`.
        let state = AppState::new(
            store,
            store_path,
            obs,
            Arc::new(runtime),
            driver.clone(),
            clock.clone(),
            Arc::new(SimDataplane::new()),
            Arc::new(SimCa::new(Arc::new(SimEntropy::new(seed)))),
            Arc::new(IdentityMgr::new(None)),
            node_id,
            allocator,
            overdrive_control_plane::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
        );
        // The production exit observer authors a crashed allocation's
        // `Failed` row from the driver's exit event (production composes it
        // in `run_server_with_obs_and_driver`).
        exit_observer::spawn(
            state.obs.clone(),
            driver.clone(),
            state.lifecycle_events.clone(),
            clock.clone(),
        );
        Self {
            _tmp: tmp,
            state,
            views,
            driver,
            clock,
            owner,
            _held_for_app_state: HeldForAppState {
                mtls_worker,
                exec_gate,
                exec_supervisor,
                guest_pool,
            },
            reconciler: ReconcilerName::new("workload-lifecycle").expect("reconciler name"),
            tick: AtomicU64::new(0),
            seed,
            trace: Mutex::new(Vec::new()),
        }
    }

    fn note(&self, line: String) {
        self.trace.lock().push(line);
    }

    fn harness_failure(&self, what: &str) -> ! {
        let trace = self.trace.lock().join("\n  ");
        panic!(
            "seed={seed}: harness precondition failed (not a contract verdict): {what}\n\
             trace:\n  {trace}\n\
             reproduce: {SEED_ENV}={seed} cargo xtask lima run -- cargo nextest run -p overdrive-sim \
             --test integration --features integration-tests --run-ignored all --no-capture \
             -E 'test(node_wide_attachment_admission_never_exceeds_the_t1_cap_across_workloads)'",
            seed = self.seed,
        );
    }

    fn workload_id(workload: &str) -> WorkloadId {
        WorkloadId::new(workload).expect("valid workload id")
    }

    /// Store effects of `handlers::submit_workload` for a fresh Job intent.
    ///
    /// Demand is 0 mCPU and 256 KiB so the existing CPU/memory check (R9,
    /// unchanged; GH #261) can never bind before the attachment cap. The
    /// baseline node has 4,000 mCPU and 8 GiB, and (CAP + 2) × 256 KiB <
    /// 8 GiB, so only the attachment cap decides admission here.
    async fn operator_deploys(&self, workload: &str) {
        let job = Job::from_submit(JobSpecInput {
            id: workload.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 0, memory_bytes: DEMAND_MEMORY_BYTES },
            driver: DriverInput::Vm(VmInput {
                command: "/bin/sleep".to_owned(),
                args: vec!["3600".to_owned()],
                kernel: "/nd295/kernel".to_owned(),
                rootfs: "/nd295/rootfs.ext4".to_owned(),
            }),
        })
        .expect("valid job spec");
        let archived =
            WorkloadIntent::Job(job.clone()).archive_for_store().expect("archive workload intent");
        let key = IntentKey::for_workload(&job.id);
        let outcome = self
            .state
            .store
            .put_if_absent(key.as_bytes(), archived.as_ref())
            .await
            .expect("intent write");
        if !matches!(outcome, PutOutcome::Inserted) {
            self.harness_failure(&format!("workload {workload} was not fresh"));
        }
        let kind_key = IntentKey::for_workload_kind(&job.id);
        self.state
            .store
            .put(kind_key.as_bytes(), &[WorkloadKind::Job.discriminator_byte()])
            .await
            .expect("kind write");
    }

    /// Store and allocator effects of `handlers::submit_workload` for a fresh
    /// Service intent: frontend address, VIP, intent, kind, listener facts.
    /// A Service is used where the proof needs a crashed allocation to be
    /// REPLACED. A crashed Job is a natural exit, which `WorkloadLifecycle`
    /// finalizes (`is_natural_exit`) instead of restarting.
    async fn operator_deploys_service(&self, workload: &str) {
        let service = Service::from_submit(ServiceSpecInput {
            id: workload.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 0, memory_bytes: DEMAND_MEMORY_BYTES },
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
        let mut facts = self.state.listener_facts.lock().await;
        facts.upsert(service.id.clone(), &vip, &service.listeners);
        drop(facts);
        self.note(format!("operator deploys service {workload}"));
    }

    /// Store effect of `handlers::stop_workload`.
    async fn operator_stops(&self, workload: &str) {
        let stop_key = IntentKey::for_workload_stop(&Self::workload_id(workload));
        self.state.store.put_if_absent(stop_key.as_bytes(), b"").await.expect("stop write");
        self.note(format!("operator stops {workload}"));
    }

    /// Store effect of `handlers::restart_workload` (a resume when stopped).
    async fn operator_restarts(&self, workload: &str) {
        let id = Self::workload_id(workload);
        let gen_key = IntentKey::for_workload_generation(&id);
        let stop_key = IntentKey::for_workload_stop(&id);
        self.state
            .store
            .txn(vec![
                TxnOp::IncrementU64 { key: Bytes::copy_from_slice(gen_key.as_bytes()) },
                TxnOp::Delete { key: Bytes::copy_from_slice(stop_key.as_bytes()) },
            ])
            .await
            .expect("restart txn");
        self.note(format!("operator restarts {workload}"));
    }

    /// One production `workload-lifecycle` evaluation for `workload`.
    async fn converge(&self, workload: &str) -> Result<(), String> {
        let n = self.tick.fetch_add(1, Ordering::SeqCst);
        let now = self.clock.now();
        let target = TargetResource::new(&format!("workload/{workload}")).expect("target");
        Box::pin(run_convergence_tick_with_guest_network_provisioner_for_test(
            &self.state,
            &self.reconciler,
            &target,
            now,
            n,
            now + Duration::from_secs(1),
            self.owner.as_ref(),
        ))
        .await
        .map_err(|error| format!("{error:?}"))
    }

    async fn rows(&self) -> BTreeMap<AllocationId, AllocStatusRow> {
        self.state
            .obs
            .alloc_status_rows()
            .await
            .expect("alloc status rows")
            .into_iter()
            .map(|row| (row.alloc_id.clone(), row))
            .collect()
    }

    async fn census(&self) -> (Census, usize, BTreeMap<AllocationId, AllocStatusRow>) {
        let rows = self.rows().await;
        let mut census = Census::default();
        for alloc in self.owner.held() {
            match rows.get(&alloc).map(|row| row.state) {
                Some(AllocState::Running) => census.running += 1,
                Some(state) if state.is_terminal() => census.terminal += 1,
                _ => census.in_flight += 1,
            }
        }
        let node_running_rows =
            rows.values().filter(|row| row.state == AllocState::Running).count();
        (census, node_running_rows, rows)
    }

    async fn expect_census(&self, expected: Census, when: &str) {
        let (census, _, _) = self.census().await;
        if census != expected {
            self.harness_failure(&format!("{when}: census {census:?} != expected {expected:?}"));
        }
        self.note(format!("{when}: census {census:?}"));
    }

    /// One placement decision for a freshly deployed workload.
    async fn probe_placement(&self, workload: &str) -> Decision {
        let (before, node_running_rows, _) = self.census().await;
        let retired_before = self.owner.retired_count();
        self.owner.begin_window();
        let journal_start = self.owner.journal_len();
        let dispatch = self.converge(workload).await;
        let (held_at_acquisition, acquired) = self.owner.window();
        let decision = Decision {
            before,
            retired_before,
            node_running_rows,
            admitted: self.owner.holds_for(workload),
            acquired,
            held_at_acquisition,
            refused: refusals(&self.owner.journal_since(journal_start)),
            dispatch,
        };
        self.note(format!("placement {workload}: {}", decision.evidence()));
        decision
    }

    /// Converge a stopped workload until its lease is released: the pool's
    /// `guest_network.lease_released` event is the only release signal.
    async fn settle_released(&self, workload: &str) {
        for _ in 0..SETTLE_TICKS {
            if !self.owner.holds_for(workload) {
                return;
            }
            if let Err(error) = self.converge(workload).await {
                self.note(format!("settle {workload}: {error}"));
            }
        }
        if self.owner.holds_for(workload) {
            self.harness_failure(&format!(
                "{workload}'s lease was never released after stop: no \
                 `{LEASE_RELEASED} {{ alloc }}` event for its allocation within {SETTLE_TICKS} \
                 evaluations (FD § \"[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)\" (the admission refusal projection and the lease events)), so the held population cannot be measured"
            ));
        }
    }

    /// Whether `workload` has a Running successor of `predecessor` and the
    /// predecessor's lease is released.
    async fn replaced(&self, workload: &str, predecessor: &AllocationId) -> bool {
        let rows = self.rows().await;
        let successor_running = rows.values().any(|row| {
            row.workload_id.as_str() == workload
                && row.state == AllocState::Running
                && row.alloc_id != *predecessor
        });
        successor_running && !self.owner.holds(predecessor)
    }

    /// Crash the workload's Running allocation through the driver exit port.
    async fn crash(&self, workload: &str) -> AllocationId {
        let rows = self.rows().await;
        let Some(alloc) = rows
            .values()
            .find(|row| row.workload_id.as_str() == workload && row.state == AllocState::Running)
            .map(|row| row.alloc_id.clone())
        else {
            self.harness_failure(&format!("{workload} has no Running allocation to crash"));
        };
        self.driver.inject_exit_after(
            &alloc,
            Duration::ZERO,
            ExitKind::Crashed { exit_code: None, signal: Some(9) },
        );
        // Every step yields to the current-thread runtime (the exit-injection
        // task and the exit observer run on it) and advances only the sim
        // clock; no wall time is read, so the step count is a function of the
        // seed and the tasks' own await points.
        for _ in 0..CRASH_POLL_BUDGET {
            tokio::task::yield_now().await;
            self.clock.tick(Duration::from_millis(1));
            let row = self.state.obs.alloc_status_row(&alloc).await.expect("row read");
            if row.is_some_and(|row| row.state == AllocState::Failed) {
                self.note(format!("crashed {workload} ({alloc}) -> Failed row by exit observer"));
                return alloc;
            }
        }
        self.harness_failure(&format!(
            "exit observer never published Failed for {alloc} within {CRASH_POLL_BUDGET} \
             yield-and-tick steps"
        ));
    }
}

// ---------------------------------------------------------------------------
// Verdicts.
// ---------------------------------------------------------------------------

struct Verdict {
    id: &'static str,
    green: bool,
    evidence: String,
}

struct Report {
    verdicts: Vec<Verdict>,
}

impl Report {
    fn verdict(&mut self, node: &SimNode, id: &'static str, green: bool, evidence: String) {
        let label = if green { "GREEN" } else { "RED" };
        node.note(format!("verdict [{label}] {id}: {evidence}"));
        eprintln!("seed={} [{label}] {id}: {evidence}", node.seed);
        self.verdicts.push(Verdict { id, green, evidence });
    }

    /// NA-VIEW (DR-12): an at-cap placement over the production
    /// `GuestAttachmentView` returns `NoCapacity` before any dispatch, so its
    /// window holds no `guest_network.admission_refused`. A refusal there
    /// means the view under-reported occupancy and the pool's own `assign`
    /// stopped the overflow instead, which NA-1 and NA-2 alone cannot tell
    /// apart from a correct placement refusal.
    fn view_verdict(&mut self, node: &SimNode, id: &'static str, decision: &Decision) {
        self.verdict(
            node,
            id,
            decision.refused.is_empty(),
            format!(
                "{ADMISSION_REFUSED} events in this at-cap placement window: [{}] (placement \
                 over the read-port must refuse before any dispatch); {}",
                render_events(&decision.refused),
                decision.evidence(),
            ),
        );
    }
}

#[derive(Debug, Clone, Copy)]
enum Block {
    Placement,
    InFlight,
    Retiring,
    Resume,
    Contended,
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-05D — Across a whole node, held attachments never exceed the cap.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER step 07-03 (S-ND295-05D)"]
async fn node_wide_attachment_admission_never_exceeds_the_t1_cap_across_workloads() {
    for seed in seeds() {
        eprintln!(
            "seed={seed} body=node_wide_attachment_admission_never_exceeds_the_t1_cap_across_workloads"
        );
        Box::pin(node_wide_admission(seed)).await;
    }
}

/// One seed of S-ND295-05D on a fresh node.
async fn node_wide_admission(seed: u64) {
    eprintln!("seed={seed} invariant=node_wide_held_attachment_admission cap={CAP}");
    let mut rng = StdRng::seed_from_u64(seed);
    let owner = Arc::new(LeaseLedger::new());
    // Thread-local capture of the pinned lease events for this seed's run.
    let _lease_events =
        tracing::subscriber::set_default(Registry::default().with(owner.event_layer()));
    let node = SimNode::compose(seed, Arc::clone(&owner)).await;

    // --- Fill: 16,384 distinct workloads, each admitted by the production
    //     reconciler evaluation, each reaching Running with one lease. ---
    let mut fillers: Vec<String> = (0..CAP).map(|index| format!("f{index:05}")).collect();
    fillers.shuffle(&mut rng);
    for (index, workload) in fillers.iter().enumerate() {
        node.operator_deploys(workload).await;
        if let Err(error) = node.converge(workload).await {
            node.harness_failure(&format!("filler {workload} evaluation failed: {error}"));
        }
        if node.owner.held_count() != index + 1 {
            node.harness_failure(&format!(
                "filler {workload} did not acquire exactly one attachment (held={})",
                node.owner.held_count()
            ));
        }
        if (index + 1) % 4_096 == 0 {
            eprintln!("seed={seed} fill progress {}/{CAP}", index + 1);
        }
    }
    let baseline = Census { running: CAP, in_flight: 0, terminal: 0 };
    node.expect_census(baseline, "after fill").await;
    node.owner.start_measuring();
    eprintln!("seed={seed} fill: {CAP} distinct workloads Running");

    // Seeded victims (distinct fillers) and block order.
    let mut victim_indices = BTreeSet::new();
    while victim_indices.len() < 4 {
        victim_indices.insert(rng.gen_range(0..CAP));
    }
    let mut victims: Vec<String> =
        victim_indices.into_iter().map(|index| fillers[index].clone()).collect();
    victims.shuffle(&mut rng);
    let (released_victim, displaced_victim, resumed_victim, contended_victim) =
        (victims[0].clone(), victims[1].clone(), victims[2].clone(), victims[3].clone());
    let mut blocks =
        [Block::Placement, Block::InFlight, Block::Retiring, Block::Resume, Block::Contended];
    blocks.shuffle(&mut rng);
    let plan = format!(
        "victims release={released_victim} displace-for-service={displaced_victim} \
         resume={resumed_victim} contended-slot={contended_victim}; block order {blocks:?}"
    );
    node.note(plan.clone());
    eprintln!("seed={seed} {plan}");

    let mut report = Report { verdicts: Vec::new() };

    for block in blocks {
        node.expect_census(baseline, &format!("before block {block:?}")).await;
        match block {
            Block::Placement => {
                node.operator_deploys("na1-probe").await;
                let decision = node.probe_placement("na1-probe").await;
                report.verdict(
                    &node,
                    "NA-1 (16,384 Running)",
                    !decision.admitted,
                    decision.evidence(),
                );
                report.view_verdict(&node, "NA-VIEW (NA-1, 16,384 Running)", &decision);
                node.operator_stops("na1-probe").await;
                node.settle_released("na1-probe").await;
            }
            Block::InFlight => {
                node.operator_stops(&released_victim).await;
                node.settle_released(&released_victim).await;
                node.expect_census(
                    Census { running: CAP - 1, in_flight: 0, terminal: 0 },
                    "after releasing one attachment",
                )
                .await;
                node.operator_deploys("na2-held").await;
                node.operator_deploys("na2-probe").await;

                // One evaluation parks after its lease was taken and before
                // its Running row exists (the in-flight window of one of the
                // eight concurrent production evaluations).
                let (entered, release) = node.owner.arm_park();
                let held = node.converge("na2-held");
                let mut held = std::pin::pin!(held);
                let parked_alloc = tokio::select! {
                    biased;
                    outcome = &mut held => {
                        node.harness_failure(&format!(
                            "na2-held evaluation finished without reaching provision: {outcome:?}"
                        ));
                    }
                    parked = entered => parked.expect("park signal"),
                };
                node.note(format!("na2-held parked in flight at provision ({parked_alloc})"));
                let held_admitted = node.owner.holds_for("na2-held");
                report.verdict(
                    &node,
                    "NA-5 (release, then in-flight admission)",
                    held_admitted,
                    format!(
                        "after {released_victim} released its attachment (held={}), a new \
                         workload's placement acquired an attachment={held_admitted} \
                         ({parked_alloc})",
                        CAP - 1
                    ),
                );
                let decision = node.probe_placement("na2-probe").await;
                report.verdict(&node, "NA-2", !decision.admitted, decision.evidence());
                report.view_verdict(&node, "NA-VIEW (NA-2, one admission in flight)", &decision);
                release.send(()).expect("parked provision is still waiting");
                if let Err(error) = held.await {
                    node.harness_failure(&format!("na2-held completion failed: {error}"));
                }
                node.operator_stops("na2-probe").await;
                node.settle_released("na2-probe").await;
            }
            Block::Retiring => {
                // Make room for one restartable Service, then crash it.
                node.operator_stops(&displaced_victim).await;
                node.settle_released(&displaced_victim).await;
                node.expect_census(
                    Census { running: CAP - 1, in_flight: 0, terminal: 0 },
                    "after releasing one attachment for the Service",
                )
                .await;
                node.operator_deploys_service("na3-svc").await;
                let placed = node.probe_placement("na3-svc").await;
                report.verdict(
                    &node,
                    "NA-5 (release, then Service placement)",
                    placed.admitted,
                    placed.evidence(),
                );
                if !placed.admitted {
                    node.harness_failure("the Service crash victim was never placed");
                }
                let crashed_alloc = node.crash("na3-svc").await;
                node.expect_census(
                    Census { running: CAP - 1, in_flight: 0, terminal: 1 },
                    "after crash (lease still held)",
                )
                .await;

                // A crashed predecessor's lease counts until its cleanup
                // finishes (R6/R7): the node is at the cap.
                node.operator_deploys("na3-probe").await;
                let decision = node.probe_placement("na3-probe").await;
                report.verdict(
                    &node,
                    "NA-1 (16,383 Running + 1 crashed predecessor still held)",
                    !decision.admitted,
                    format!("crashed predecessor {crashed_alloc}; {}", decision.evidence()),
                );
                report.view_verdict(
                    &node,
                    "NA-VIEW (NA-1, 16,383 Running + 1 crashed predecessor still held)",
                    &decision,
                );

                // Replace the crashed Service. At the cap the due restart
                // reclaims the predecessor first (R11); the window covers
                // every evaluation of the replacement.
                let journal_start = node.owner.journal_len();
                let held_before = node.owner.held_count();
                node.owner.begin_window();
                let mut dispatches = vec![node.converge("na3-svc").await];
                // Only reachable when NA-1 above was violated: free the
                // probe's lease so the liveness guard below stays usable.
                if node.owner.holds_for("na3-probe") {
                    node.operator_stops("na3-probe").await;
                    node.settle_released("na3-probe").await;
                }
                for _ in 0..SETTLE_TICKS {
                    if node.replaced("na3-svc", &crashed_alloc).await {
                        break;
                    }
                    dispatches.push(node.converge("na3-svc").await);
                }
                let (held_peak, acquired) = node.owner.window();
                let (after, _, rows) = node.census().await;
                report.verdict(
                    &node,
                    "NA-4a",
                    held_peak.is_none_or(|peak| peak <= CAP),
                    format!(
                        "replacement of crashed Service na3-svc ({crashed_alloc}) from held=\
                         {held_before}: acquired {acquired:?}; peak held population at those \
                         acquisitions={held_peak:?}; after: {after:?} (held {}); dispatches=\
                         {dispatches:?}",
                        after.held()
                    ),
                );
                let (ordered, evidence) = recreate_order(
                    &node.owner.journal_since(journal_start),
                    "na3-svc",
                    &crashed_alloc,
                );
                report.verdict(
                    &node,
                    "NA-RECREATE",
                    ordered,
                    format!("held when the replacement began={held_before}; {evidence}"),
                );
                let running_successor: Vec<&AllocationId> = rows
                    .values()
                    .filter(|row| {
                        row.workload_id.as_str() == "na3-svc"
                            && row.state == AllocState::Running
                            && row.alloc_id != crashed_alloc
                    })
                    .map(|row| &row.alloc_id)
                    .collect();
                let predecessor_held = node.owner.holds(&crashed_alloc);
                report.verdict(
                    &node,
                    "NA-4a-L (liveness: crashed Service is replaced once capacity exists)",
                    !running_successor.is_empty() && !predecessor_held,
                    format!(
                        "Running successor(s) {running_successor:?}; crashed {crashed_alloc} \
                         still holds a lease={predecessor_held}; census {after:?}"
                    ),
                );
            }
            Block::Contended => {
                // NA-E7-C: one free slot; two new workloads are both placed on
                // it before either dispatches. The first evaluation parks
                // between its placement and its dispatch (its view
                // write-through); the second runs whole and takes the slot.
                node.operator_stops(&contended_victim).await;
                node.settle_released(&contended_victim).await;
                node.expect_census(
                    Census { running: CAP - 1, in_flight: 0, terminal: 0 },
                    "after releasing the contended slot",
                )
                .await;
                node.operator_deploys("na5-parked").await;
                node.operator_deploys("na5-winner").await;
                let journal_start = node.owner.journal_len();
                let (entered, release) = node
                    .views
                    .arm_park(TargetResource::new("workload/na5-parked").expect("target"));
                let parked = node.converge("na5-parked");
                let mut parked = std::pin::pin!(parked);
                tokio::select! {
                    biased;
                    outcome = &mut parked => {
                        node.harness_failure(&format!(
                            "na5-parked's evaluation finished without reaching its view \
                             write-through, so it was never placed before dispatch: {outcome:?}"
                        ));
                    }
                    signal = entered => signal.expect("view-park signal"),
                }
                node.note(
                    "na5-parked placed on the free slot and parked before dispatch".to_owned(),
                );
                let winner = node.converge("na5-winner").await;
                let winner_admitted = node.owner.holds_for("na5-winner");
                release.send(()).expect("the parked view write-through is still waiting");
                let parked_dispatch = parked.await;
                let window = refusals(&node.owner.journal_since(journal_start));
                let parked_refusals = window
                    .iter()
                    .filter(|event| {
                        matches!(
                            event,
                            LeaseEvent::Refused { alloc: Some(alloc), .. }
                                if workload_of(alloc) == Some("na5-parked")
                        )
                    })
                    .count();
                report.verdict(
                    &node,
                    "NA-E7-C (witness: the contended slot's loser is refused exactly once)",
                    winner_admitted
                        && !node.owner.holds_for("na5-parked")
                        && window.len() == 1
                        && parked_refusals == 1,
                    format!(
                        "na5-winner admitted={winner_admitted} (dispatch {winner:?}); na5-parked \
                         holds a lease={}; its dispatch after the winner: {parked_dispatch:?}; \
                         {ADMISSION_REFUSED} in the window: [{}]",
                        node.owner.holds_for("na5-parked"),
                        render_events(&window),
                    ),
                );

                // The runtime's immediate re-evaluation of the refused
                // workload sees the node at the cap: placement refuses and
                // nothing is dispatched, so no second refusal is journaled.
                let reevaluation = node.converge("na5-parked").await;
                let after = refusals(&node.owner.journal_since(journal_start));
                report.verdict(
                    &node,
                    "NA-E7-C (no second refused dispatch for the contended slot)",
                    after.len() == window.len() && !node.owner.holds_for("na5-parked"),
                    format!(
                        "re-evaluation of na5-parked: {reevaluation:?}; {ADMISSION_REFUSED} since \
                         the contention began: [{}]",
                        render_events(&after),
                    ),
                );
                node.operator_stops("na5-parked").await;
            }
            Block::Resume => {
                // A restartable Service takes the freed attachment and is
                // then operator-stopped. A Job refills the node, and the
                // operator resumes the Service at the cap. (A stopped Job
                // is not a usable resume victim: the action shim drops a
                // RestartAllocation whose Job predecessor carries a terminal
                // claim, see `allocation_attempt_transition`.)
                node.operator_stops(&resumed_victim).await;
                node.settle_released(&resumed_victim).await;
                node.expect_census(
                    Census { running: CAP - 1, in_flight: 0, terminal: 0 },
                    "after operator stop released one attachment",
                )
                .await;
                node.operator_deploys_service("na4-svc").await;
                let service_placed = node.probe_placement("na4-svc").await;
                if !service_placed.admitted {
                    node.harness_failure("the Service resume victim was never placed");
                }
                node.operator_stops("na4-svc").await;
                node.settle_released("na4-svc").await;
                node.operator_deploys("na4-filler").await;
                let refill = node.probe_placement("na4-filler").await;
                report.verdict(
                    &node,
                    "NA-5 (release, then placement)",
                    refill.admitted,
                    format!("after na4-svc released its attachment: {}", refill.evidence()),
                );
                if !refill.admitted {
                    node.harness_failure("the refill below the cap was never placed");
                }

                // Resume at the cap, then free the refill's lease and let the
                // resume complete. The window covers every evaluation of the
                // resume.
                node.operator_restarts("na4-svc").await;
                let held_before = node.owner.held_count();
                node.owner.begin_window();
                let mut dispatches = vec![node.converge("na4-svc").await];
                node.operator_stops("na4-filler").await;
                node.settle_released("na4-filler").await;
                for _ in 0..SETTLE_TICKS {
                    if node.owner.holds_for("na4-svc") {
                        break;
                    }
                    dispatches.push(node.converge("na4-svc").await);
                }
                let (held_peak, acquired) = node.owner.window();
                let (settled, _, rows) = node.census().await;
                report.verdict(
                    &node,
                    "NA-4b",
                    held_peak.is_none_or(|peak| peak <= CAP),
                    format!(
                        "operator resume of Service na4-svc from held={held_before}: acquired \
                         {acquired:?}; peak held population at those acquisitions=\
                         {held_peak:?}; after: {settled:?} (held {}); dispatches={dispatches:?}",
                        settled.held()
                    ),
                );
                let resumed: Vec<&AllocationId> = rows
                    .values()
                    .filter(|row| {
                        row.workload_id.as_str() == "na4-svc" && row.state == AllocState::Running
                    })
                    .map(|row| &row.alloc_id)
                    .collect();
                report.verdict(
                    &node,
                    "NA-4b-L (liveness: stopped Service resumes once capacity exists)",
                    !resumed.is_empty(),
                    format!(
                        "after the refill's attachment was released: Running na4-svc \
                         allocation(s) {resumed:?}; census {settled:?}"
                    ),
                );
                if resumed.is_empty() {
                    // Keep the block's end-state precondition: put the node
                    // back at 16,384 with a fresh placement.
                    node.operator_deploys("na4-refill").await;
                    if let Err(error) = node.converge("na4-refill").await {
                        node.note(format!("resume fallback refill: {error}"));
                    }
                }
            }
        }
        node.expect_census(baseline, &format!("after block {block:?}")).await;
    }

    let unaccounted = node.owner.unaccounted();
    if !unaccounted.is_empty() {
        node.harness_failure(&format!(
            "lease events the ledger cannot account for (malformed, a release of a lease never \
             provisioned, or a refusal naming no allocation): {}",
            render_events(&unaccounted)
        ));
    }

    // E7 (H13): at most one refused dispatch per contended slot, counted over
    // the whole run per allocation and per workload.
    let run_refusals = refusals(&node.owner.journal_since(0));
    let (per_alloc, per_workload) = refusal_counts(&run_refusals);
    let worst_alloc = per_alloc.values().copied().max().unwrap_or(0);
    let worst_workload = per_workload.values().copied().max().unwrap_or(0);
    report.verdict(
        &node,
        "NA-E7 (at most one refused dispatch per contended slot)",
        worst_alloc <= 1 && worst_workload <= 1,
        format!(
            "{ADMISSION_REFUSED} over the run: {} event(s); per allocation {per_alloc:?} (max \
             {worst_alloc}); per workload {per_workload:?} (max {worst_workload}); events: [{}]",
            run_refusals.len(),
            render_events(&run_refusals),
        ),
    );

    let (held_peak, held_peak_at, lease_peak, overlap_peak) = node.owner.peaks();
    report.verdict(
        &node,
        "NA-G",
        held_peak <= CAP,
        format!(
            "peak held population at any lease acquisition after the fill={held_peak} (reached \
             at {held_peak_at:?}); the fill is bounded to {CAP} by construction"
        ),
    );
    report.verdict(
        &node,
        "NA-OVERLAP",
        lease_peak <= CAP && overlap_peak.as_ref().is_none_or(|(peak, _)| *peak <= CAP),
        format!(
            "peak held population over the run, retiring predecessors and their replacements \
             included={lease_peak}; peak held at an acquisition made while the same workload's \
             predecessor still held its lease={overlap_peak:?}; cap={CAP}"
        ),
    );

    let table = report
        .verdicts
        .iter()
        .map(|entry| {
            format!(
                "  [{}] {}: {}\n",
                if entry.green { "GREEN" } else { "RED" },
                entry.id,
                entry.evidence
            )
        })
        .collect::<Vec<_>>()
        .concat();
    eprintln!("seed={seed} node-wide held-attachment admission verdicts:\n{table}");
    let red: Vec<&str> =
        report.verdicts.iter().filter(|entry| !entry.green).map(|entry| entry.id).collect();
    assert!(
        red.is_empty(),
        "seed={seed}: node-wide held-attachment admission contract violated: {red:?}\n\
         verdicts:\n{table}\
         reproduce: {SEED_ENV}={seed} cargo xtask lima run -- cargo nextest run -p overdrive-sim \
         --test integration --features integration-tests --run-ignored all --no-capture \
         -E 'test(node_wide_attachment_admission_never_exceeds_the_t1_cap_across_workloads)'"
    );
}
