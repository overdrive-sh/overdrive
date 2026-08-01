//! Tier-3 acceptance for the transparent-mTLS intercept-install FAIL-CLOSED
//! CALL-SITE ORDERING (GH #250 / ADR-0076 § 5.2; DISTILL S-MIF-04 `@keystone`
//! + S-MIF-05).
//!
//! Drives the PRODUCTION driving port `action_shim::dispatch` with a
//! `SimMtlsIntercept` armed to refuse the leg-F bind, on BOTH arms that carry
//! the fail-closed guard — `StartAllocation` (`mod.rs:1294-1308`) and
//! `RestartAllocation` (`:1494-1508`).
//!
//! # Why this test exists — and why the port exists
//!
//! The gate-non-release is a property of the CALL SITE's `return` placement,
//! not of the helper it delegates to: each arm returns from
//! `fail_closed_on_mtls_install` BEFORE reaching
//! `driver.release_for_exit_emission(handle)` / `driver.on_alloc_running(&spec)`
//! a few lines below, so a now-`Failed` allocation never releases its exit
//! watcher. A reordering that released first would survive the default-lane
//! helper contract (step 01-01) ENTIRELY — a helper-level test structurally
//! cannot observe where its caller returns. Nothing in the tree could make
//! `MtlsInterceptWorker::start_alloc` fail on demand before the
//! `MtlsIntercept` port landed; making this property assertable is the port's
//! ONE justification.
//!
//! Both arms get their OWN test function and are deliberately NOT collapsed:
//! the two production blocks are byte-identical TODAY, so a single case would
//! defend only the shared helper. What these defend against is a FUTURE
//! DIVERGENT EDIT to one block.
//!
//! # The four assertions, and why A-1' is parked
//!
//! The design pins four observables per arm (§ 5.2 / OQ-7):
//!
//! | | Assertion | Home |
//! |---|---|---|
//! | A-6' | `release_for_exit_emission` NEVER called | live, below |
//! | A-8' | `driver.on_alloc_running` never called | live, below |
//! | A-9' | the alloc STILL holds its net slot | live, below |
//! | A-1' | `Running` written FIRST, then superseded by `Failed` | `#[ignore]`d, below |
//!
//! A-1' is **`#[ignore]`d against a REAL, REPRODUCED PRODUCTION DEFECT**, not
//! deleted and not weakened — the same discipline `dns_responder_nxdomain`
//! applies to its blocked recovery-after-stop observable. The defect:
//!
//! > `fail_closed_on_mtls_install` builds its superseding `Failed` row from the
//! > SAME `tick` and the SAME `node_id` as the `Running` row it must supersede
//! > (`action_shim/mod.rs:1239` and `:430`, both resolving through
//! > `timestamp_for` at `:1728`), so both rows carry a BYTE-IDENTICAL
//! > `LogicalTimestamp { counter: tick.tick + 1, writer: node_id }`.
//! > `LogicalTimestamp::dominates` returns `false` on an equal counter with an
//! > equal writer, so the `Failed` row LOSES the LWW merge and is silently
//! > dropped — by `SimObservationStore::apply_alloc_status` AND by the
//! > production single-node `overdrive-store-local::apply_alloc_status_lww`
//! > that `run_server` wires via `wire_single_node_observation`.
//!
//! The operator-visible consequence under a real `serve` + `deploy`: an mTLS
//! intercept-install failure leaves the allocation **durably recorded
//! `Running` with no interception installed**. The driver IS stopped and the
//! `LifecycleEvent` IS emitted (both verified), so the workload is not left
//! running uninstrumented — but the durable record lies, which is the surface
//! this feature exists to defend.
//!
//! Fixing it is a production behaviour change, which ADR-0076 § Decision 4 and
//! this step's scope both exclude. UN-IGNORE A-1' in the same commit that
//! lands the fix; it passes the moment the `Failed` row dominates. A tracking
//! issue is pending operator approval (agents do not open issues unilaterally)
//! — this module doc is the interim record.
//!
//! # SUT state machine
//!
//! ```text
//!   Pending --provision netns--> spec{netns,host_veth,workload_addr} set
//!       |
//!       +-- driver.start Ok --> Running(row written, watcher parked)
//!               |
//!               +-- start_alloc Ok  --> release_for_exit_emission, on_alloc_running
//!               |
//!               +-- start_alloc Err --> [fail_closed_on_mtls_install]
//!                                          stop driver (best effort)
//!                                          write superseding Failed row
//!                                          emit LifecycleEvent
//!                                          RETURN — gate never released,
//!                                          on_alloc_running never fired,
//!                                          netns + net slot still HELD
//! ```
//!
//! The retained netns/slot on the last edge is what A-9' characterises: the
//! fail-closed path returns before `teardown_and_release_netns`, and the later
//! terminal arm reaps it. Changing that is out of GH #250's scope; pinning it
//! makes such a change deliberate and visible.
//!
//! # Universe (port-exposed observables only)
//!
//! Every `AllocStatusRow` written for the alloc IN WRITE ORDER, read off the
//! `ObservationStore::subscribe_all_events` stream (the LWW point-lookup
//! collapses a supersession to its winner, so the live subscription is the only
//! surface on which "Running FIRST, then Failed" is OBSERVED rather than
//! inferred); `RecordingDriver::{releases, on_alloc_running_calls}`;
//! `NetSlotAllocator::snapshot()` keyed by `AllocationId` (a documented public
//! read-only observer). Nothing reads a `SimMtlsIntercept` fault slot — the
//! armed fault is observed only through the dispatch's port-exposed outcome.
//!
//! # Root is STRUCTURAL, not incidental
//!
//! `provision_and_inject_netns` short-circuits ONLY on `mtls_worker.is_none()`,
//! so arming the mTLS seam unavoidably reaches `provision_workload_netns` and
//! real `ip netns` shell-outs. WITHOUT root the alloc is driven `Failed` by the
//! SIBLING netns-provision handler carrying `WorkloadNetnsProvisionFailed`,
//! `Driver::start` is never called, and `start_alloc` is never reached — the
//! scenario would silently exercise the wrong handler. Every test here
//! therefore SKIPs (not fails) off root and prints an explicit EXECUTED marker
//! past the gate, so a skipped run is never mistaken for a pass.
//!
//! Each test drives a DISTINCT net slot (and therefore a distinct
//! `ovd-ns-<slot>`) so the four can run in parallel without racing on one real
//! netns. Run via `cargo xtask lima run -- cargo nextest run
//! -p overdrive-control-plane --features integration-tests`. NEVER `--no-run`.
//!
//! Cleanup: a `NetnsGuard` RAII teardown plus an explicit pre-sweep at each use
//! site, mirroring `alloc_netns_lifecycle.rs` — this repo has a documented
//! cross-run leak-hazard class for exactly this shape.

#![cfg(target_os = "linux")]
// Skip-on-no-privilege and executed-marker messages are the legitimate way
// these Tier-3 tests communicate their lane to the test log.
#![allow(clippy::print_stderr)]
// A-1'/A-6'/A-8'/A-9' etc. read as prose labels in the scenario docs, not code.
#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use tokio::sync::broadcast;

use overdrive_control_plane::action_shim::dispatch;
use overdrive_control_plane::veth_provisioner::{
    NetSlot, NetSlotAllocator, WorkloadNetnsPlan, derive_workload_netns_plan,
    responder_addr_for_slot, teardown_workload_netns,
};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, Resources,
};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LagAwareSubscription, LogicalTimestamp, ObservationRow,
    ObservationStore, SubscriptionEvent,
};
use overdrive_core::transition_reason::TransitionReason;

use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_sim::adapters::SimIdentityRead;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
use overdrive_sim::adapters::mtls_intercept::{SimInterceptFault, SimMtlsIntercept};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::mtls_intercept_port::MtlsIntercept;
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixture — every builder mirrors the sibling `alloc_netns_lifecycle.rs`, which
// is the only other test driving `dispatch` with `mtls_worker: Some(..)`.
// ---------------------------------------------------------------------------

/// True iff this process is uid 0 (root). The netns provision the mTLS seam
/// forces shells out to `ip netns add`, which needs CAP_NET_ADMIN.
fn is_root() -> bool {
    // SAFETY: getuid is always safe; it takes no args and never fails.
    unsafe { libc::getuid() == 0 }
}

/// A real `MtlsInterceptWorker` over a CALLER-SUPPLIED `MtlsIntercept`.
///
/// The sibling's own `build_worker()` takes no arguments and hard-wires
/// `HostMtlsIntercept`, leaving the caller no handle on which to arm a fault —
/// hence this test-local declaration rather than widening the sibling's
/// signature. The body below is otherwise the sibling's verbatim
/// (`alloc_netns_lifecycle.rs:110-125`): the same `SimIdentityRead` /
/// `SimMtlsEnforcement` / `SimMtlsResolve` / `SimClock` construction with their
/// real required arguments, with `intercept` as the 4th argument.
fn build_worker(intercept: Arc<dyn MtlsIntercept>) -> Arc<MtlsInterceptWorker> {
    let identity: Arc<dyn IdentityRead> = Arc::new(SimIdentityRead::new(BTreeMap::new(), None));
    let enforcement: Arc<dyn MtlsEnforcement> =
        Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()));
    let resolve: Arc<dyn overdrive_core::traits::mtls_resolve::MtlsResolve> =
        Arc::new(overdrive_sim::adapters::SimMtlsResolve::new(
            std::collections::BTreeMap::new(),
            overdrive_core::traits::mtls_resolve::MtlsResolution::NonMesh,
        ));
    Arc::new(MtlsInterceptWorker::new(
        enforcement,
        resolve,
        Arc::new(SimClock::new()),
        intercept,
    ))
}

/// A shared in-process `SimObservationStore` — the dispatch path writes the
/// alloc rows here; the assertions read them off its subscription surface.
fn build_obs() -> Arc<SimObservationStore> {
    Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0))
}

/// A VIP allocator the dispatch signature requires but neither arm under test
/// touches — a one-address pool suffices.
fn build_vip_allocator(
    store: Arc<dyn overdrive_core::traits::intent_store::IntentStore>,
) -> Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>> {
    let cidr = ipnet::Ipv4Net::new(Ipv4Addr::new(10, 96, 0, 1), 32).expect("/32 prefix");
    let range = VipRange::new(vec![cidr], std::collections::BTreeSet::new()).expect("vip range");
    Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(range, store)))
}

fn tick_now() -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000)),
        tick: 0,
        deadline: now + Duration::from_secs(120),
    }
}

fn build_spec(alloc: &AllocationId) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: overdrive_core::SpiffeId::new("spiffe://overdrive.local/workload/mif/alloc/01")
            .expect("valid spiffe id"),
        command: "/bin/true".to_owned(),
        args: Vec::new(),
        resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        // The C3 provision seam SETS these — supplied `None` so the seam's own
        // assign/provision/inject runs for real.
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
    }
}

/// RAII teardown — runs the production `teardown_workload_netns` for the
/// slot-derived plan on drop so the netns + host veth leave no residue even
/// when an assertion panics mid-test. Idempotent (teardown swallows "absent").
struct NetnsGuard {
    plan: WorkloadNetnsPlan,
}

impl Drop for NetnsGuard {
    fn drop(&mut self) {
        let _ = teardown_workload_netns(&self.plan);
    }
}

/// Pre-sweep any residue from a crashed prior run and arm the RAII guard for
/// `slot`. Each test owns a DISTINCT slot so the four can run in parallel
/// without racing on one real `ovd-ns-<slot>`.
fn arm_netns_guard(slot: NetSlot) -> NetnsGuard {
    let plan = derive_workload_netns_plan(slot, responder_addr_for_slot(slot));
    let _ = teardown_workload_netns(&plan);
    NetnsGuard { plan }
}

/// An allocator whose smallest-free slot is `slot_index`, reached by parking
/// that many holder allocs. Gives each test its own netns without any shared
/// mutable fixture.
fn allocator_positioned_at(slot_index: u16) -> NetSlotAllocator {
    let allocator = NetSlotAllocator::new();
    for i in 0..slot_index {
        let holder = AllocationId::new(&format!("mif-holder-{i}")).expect("valid alloc id");
        allocator.assign(holder).expect("holder assigns are under capacity");
    }
    allocator
}

// ---------------------------------------------------------------------------
// RecordingDriver — the step 01-01 shape, here WRAPPING a `SimDriver` so
// `Driver::start` SUCCEEDS (the alloc must reach Running for the fail-closed
// guard to be reachable at all) while `release_for_exit_emission` and
// `on_alloc_running` — both DEFAULTED no-ops on the trait — are recorded, so
// A-6' and A-8' are observable WITHOUT adding any accessor to `overdrive-sim`.
// ---------------------------------------------------------------------------

struct RecordingDriver {
    inner: SimDriver,
    releases: parking_lot::Mutex<Vec<AllocationId>>,
    on_alloc_running_calls: parking_lot::Mutex<Vec<AllocationId>>,
}

impl RecordingDriver {
    fn new() -> Self {
        Self {
            inner: SimDriver::new(DriverType::Exec),
            releases: parking_lot::Mutex::new(Vec::new()),
            on_alloc_running_calls: parking_lot::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl Driver for RecordingDriver {
    fn r#type(&self) -> DriverType {
        self.inner.r#type()
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        self.inner.start(spec).await
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        self.inner.stop(handle).await
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

    fn release_for_exit_emission(&self, handle: &AllocationHandle) {
        self.releases.lock().push(handle.alloc.clone());
        self.inner.release_for_exit_emission(handle);
    }

    fn on_alloc_running(&self, spec: &AllocationSpec) {
        self.on_alloc_running_calls.lock().push(spec.alloc.clone());
        self.inner.on_alloc_running(spec);
    }
}

/// Drive a single `Action` through the production `action_shim::dispatch` with
/// the supplied `driver` + `net_slot_allocator` + a REAL `MtlsInterceptWorker`
/// (so the C3 provision seam AND the mTLS install seam are both ARMED). Every
/// orthogonal port is a sim double.
async fn dispatch_one(
    action: Action,
    driver: &dyn Driver,
    obs: &dyn ObservationStore,
    store: Arc<dyn overdrive_core::traits::intent_store::IntentStore>,
    worker: &Arc<MtlsInterceptWorker>,
    net_slot_allocator: &NetSlotAllocator,
) -> Result<(), overdrive_control_plane::action_shim::ShimError> {
    let dataplane: Arc<dyn overdrive_core::traits::dataplane::Dataplane> =
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new());
    let (lifecycle_tx, _lifecycle_rx) = broadcast::channel(64);
    let writer_node = NodeId::new("writer-1").expect("NodeId");
    let tick = tick_now();
    let broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    dispatch(
        vec![action],
        driver,
        obs,
        dataplane.as_ref(),
        &overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        )),
        &SimClock::new(),
        &overdrive_control_plane::identity_mgr::IdentityMgr::new(None),
        &lifecycle_tx,
        &tick,
        &writer_node,
        build_vip_allocator(store),
        &broker,
        None,
        Some(worker),
        net_slot_allocator,
    )
    .await
}

// ---------------------------------------------------------------------------
// Universe readers
// ---------------------------------------------------------------------------

/// Every `AllocStatusRow` written for `alloc`, IN WRITE ORDER.
///
/// The LWW point-lookup (`alloc_status_row`) collapses a supersession to its
/// winner, so it cannot show "Running FIRST, then Failed". The live
/// `subscribe_all_events` stream is the port-exposed surface that CAN: it
/// yields each ACCEPTED write in the order it landed. The subscription must be
/// opened BEFORE the dispatch or the writes are missed.
async fn drain_alloc_rows(
    subscription: &mut LagAwareSubscription,
    alloc: &AllocationId,
) -> Vec<AllocStatusRow> {
    let mut rows = Vec::new();
    // The dispatch has already returned, so every accepted write is buffered on
    // the broadcast channel; the short timeout is the drain terminator, not a
    // liveness wait.
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(250), subscription.next()).await
    {
        match event {
            SubscriptionEvent::Row(ObservationRow::AllocStatus(row)) => {
                if &row.alloc_id == alloc {
                    rows.push(*row);
                }
            }
            SubscriptionEvent::Row(_) => {}
            // A handful of writes against the fan-out cannot lag; a `Lagged`
            // here is a real bug, never something to swallow.
            SubscriptionEvent::Lagged { missed } => {
                panic!(
                    "observation subscription lagged ({missed} rows missed) — the write-order \
                     universe is incomplete"
                );
            }
        }
    }
    rows
}

/// Everything the assertions read, produced by ONE fail-closed dispatch.
struct FailClosedOutcome {
    /// A-1' — every accepted `AllocStatusRow` for the alloc, in write order.
    rows: Vec<AllocStatusRow>,
    /// A-6' — allocs `release_for_exit_emission` was called for.
    releases: Vec<AllocationId>,
    /// A-8' — allocs `on_alloc_running` was called for.
    on_alloc_running_calls: Vec<AllocationId>,
    /// A-9' — whether the alloc still holds its net slot afterwards.
    slot_still_held: bool,
}

/// Which production arm to drive. The two carry byte-identical guard blocks
/// TODAY; keeping them as separate drives is what defends against a FUTURE
/// DIVERGENT EDIT to one of them.
#[derive(Clone, Copy)]
enum Arm {
    Start,
    Restart,
}

/// Seed a prior `Running` row so the `RestartAllocation` arm's
/// `find_prior_alloc_row` resolves `(workload_id, node_id)`.
///
/// `counter: 0` so the restart's own write (`tick.tick + 1`) strictly dominates
/// under LWW.
async fn seed_running_row(
    obs: &dyn ObservationStore,
    alloc: &AllocationId,
    workload: &WorkloadId,
    node: &NodeId,
) {
    let row = AllocStatusRow {
        alloc_id: alloc.clone(),
        workload_id: workload.clone(),
        node_id: node.clone(),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 0, writer: node.clone() },
        reason: Some(TransitionReason::Started),
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
    };
    obs.write(ObservationRow::AllocStatus(Box::new(row)))
        .await
        .expect("seed prior Running alloc row");
}

/// Drive `arm` through the real `dispatch` with the leg-F bind refused, on a
/// dedicated `slot_index`, and collect every port-exposed observable.
///
/// The `Arc::clone` BEFORE the cast is load-bearing: the test retains a typed
/// `Arc<SimMtlsIntercept>` so it can arm (and if needed `clear_faults()`) after
/// the worker holds its `Arc<dyn MtlsIntercept>`.
async fn drive_fail_closed(arm: Arm, slot_index: u16, alloc_name: &str) -> FailClosedOutcome {
    let tmp = TempDir::new().expect("tempdir");
    let store: Arc<dyn overdrive_core::traits::intent_store::IntentStore> =
        Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open store"));
    let obs = build_obs();

    let intercept = Arc::new(SimMtlsIntercept::new());
    intercept.script_bind_fault(SimInterceptFault::TransparentListener { errno: libc::EPERM });
    let worker = build_worker(Arc::clone(&intercept) as Arc<dyn MtlsIntercept>);

    let driver = RecordingDriver::new();
    let allocator = allocator_positioned_at(slot_index);
    let _guard = arm_netns_guard(NetSlot::new(slot_index).expect("slot in range"));

    let alloc = AllocationId::new(alloc_name).expect("valid alloc id");
    let workload =
        WorkloadId::new(&format!("svc-{alloc_name}")).expect("valid workload id");
    let node = NodeId::new("node-001").expect("valid node id");

    let action = match arm {
        Arm::Start => Action::StartAllocation {
            alloc_id: alloc.clone(),
            workload_id: workload.clone(),
            node_id: node.clone(),
            spec: build_spec(&alloc),
            kind: WorkloadKind::Service,
        },
        Arm::Restart => {
            // Fixture delta: the restart arm resolves the alloc's identity off
            // a prior row. Seeded BEFORE the subscription so the seed write is
            // not part of the asserted write-order universe.
            seed_running_row(obs.as_ref(), &alloc, &workload, &node).await;
            Action::RestartAllocation {
                alloc_id: alloc.clone(),
                spec: build_spec(&alloc),
                kind: WorkloadKind::Service,
                reason: None,
            }
        }
    };

    // Opened BEFORE the dispatch — the write-order universe is only observable
    // on a subscription that predates the writes.
    let mut subscription: LagAwareSubscription =
        obs.subscribe_all_events().await.expect("subscribe to the observation store");

    dispatch_one(action, &driver, obs.as_ref(), Arc::clone(&store), &worker, &allocator)
        .await
        .expect(
            "the install failure must be RECORDED and the dispatch return Ok — a bubbled Err is \
             the indefinite-Pending-retry regression",
        );

    let rows = drain_alloc_rows(&mut subscription, &alloc).await;
    let outcome = FailClosedOutcome {
        rows,
        releases: driver.releases.lock().clone(),
        on_alloc_running_calls: driver.on_alloc_running_calls.lock().clone(),
        slot_still_held: allocator.snapshot().contains_key(&alloc),
    };

    worker.stop_alloc(&alloc);
    outcome
}

// ---------------------------------------------------------------------------
// A-6' / A-8' / A-9' — the CALL-SITE ORDERING properties. The port's ONE
// justification, and the litmus-proven core of this step.
// ---------------------------------------------------------------------------

/// A-6' (the security-critical one), A-8', A-9'.
///
/// A-6' is a property of the call site's `return` placement — the arm returns
/// BEFORE `driver.release_for_exit_emission(handle)` a few lines below — so a
/// reordering that released first survives the helper-level contract entirely.
/// This assertion, and only this one, dies on that reordering.
fn assert_ordering_observables(scenario: &str, outcome: &FailClosedOutcome) {
    assert!(
        outcome.releases.is_empty(),
        "{scenario} A-6': a now-Failed allocation must NEVER release its exit watcher — the \
         fail-closed arm must return BEFORE driver.release_for_exit_emission, got releases {:?}",
        outcome.releases,
    );
    assert!(
        outcome.on_alloc_running_calls.is_empty(),
        "{scenario} A-8': a now-Failed allocation must never be announced running to its driver — \
         the same ordering property, second observable — got {:?}",
        outcome.on_alloc_running_calls,
    );
    // A characterisation of today's behaviour: the fail-closed path returns
    // before `teardown_and_release_netns`, so the netns and slot stay held and
    // the later terminal arm reaps them.
    assert!(
        outcome.slot_still_held,
        "{scenario} A-9': the fail-closed path returns before teardown_and_release_netns, so the \
         alloc must STILL hold its net slot (reaped later by the terminal arm)",
    );
}

/// S-MIF-04 (`@keystone`) — a failed intercept install on a FRESH allocation
/// keeps its exit watcher. Defends the `StartAllocation` guard's
/// `return`-before-release placement (`mod.rs:1297-1307` before `:1309`).
#[tokio::test]
async fn start_allocation_install_failure_never_releases_the_exit_watcher() {
    if !is_root() {
        eprintln!(
            "SKIP start_allocation_install_failure_never_releases_the_exit_watcher: not root — \
             the mTLS seam forces a real netns provision, and off root the SIBLING \
             WorkloadNetnsProvisionFailed handler fires instead (wrong handler, nothing asserted)"
        );
        return;
    }
    eprintln!("EXECUTED S-MIF-04 (root): driving the real StartAllocation fail-closed arm");

    let outcome = drive_fail_closed(Arm::Start, 0, "mif-start").await;

    assert_ordering_observables("S-MIF-04", &outcome);
}

/// S-MIF-05 — a failed intercept install on a RESTARTED allocation keeps its
/// exit watcher. Defends the `RestartAllocation` guard's
/// `return`-before-release placement (`mod.rs:1497-1507` before `:1509`).
///
/// The two production blocks are byte-identical TODAY, so what this adds over
/// S-MIF-04 is a defense against a FUTURE DIVERGENT EDIT to one of them — the
/// real risk, and why the two are not collapsed.
#[tokio::test]
async fn restart_allocation_install_failure_never_releases_the_exit_watcher() {
    if !is_root() {
        eprintln!(
            "SKIP restart_allocation_install_failure_never_releases_the_exit_watcher: not root — \
             the mTLS seam forces a real netns provision, and off root the SIBLING \
             WorkloadNetnsProvisionFailed handler fires instead (wrong handler, nothing asserted)"
        );
        return;
    }
    eprintln!("EXECUTED S-MIF-05 (root): driving the real RestartAllocation fail-closed arm");

    let outcome = drive_fail_closed(Arm::Restart, 1, "mif-restart").await;

    assert_ordering_observables("S-MIF-05", &outcome);
}

// ---------------------------------------------------------------------------
// A-1' — the ROW SUPERSESSION property. BLOCKED on a reproduced production
// defect; see the module doc for the full diagnosis. Un-ignore in the same
// commit that lands the fix.
// ---------------------------------------------------------------------------

/// A-1' — the `Running` row is written FIRST and then SUPERSEDED by a `Failed`
/// row carrying `MtlsInterceptInstallFailed { stage: "leg_f_bind", .. }`.
///
/// Proves `start_alloc`'s `Err` reaches the helper THROUGH the production
/// guard, which no helper-level test can establish.
fn assert_supersession_observable(scenario: &str, outcome: &FailClosedOutcome) {
    let rows = &outcome.rows;
    assert_eq!(
        rows.len(),
        2,
        "{scenario} A-1': the dispatch must write exactly two rows for the alloc — a Running row \
         and the Failed row that supersedes it — got {rows:?}",
    );
    assert_eq!(
        rows[0].state,
        AllocState::Running,
        "{scenario} A-1': the FIRST row written must be Running (the alloc reached Running before \
         the intercept install was attempted), got {:?} ({:?})",
        rows[0].state,
        rows[0].reason,
    );
    assert_eq!(
        rows[1].state,
        AllocState::Failed,
        "{scenario} A-1': the Running row must be SUPERSEDED by a Failed row, got {:?} ({:?})",
        rows[1].state,
        rows[1].reason,
    );
    assert!(
        matches!(
            rows[1].reason,
            Some(TransitionReason::MtlsInterceptInstallFailed { ref stage, .. })
                if stage == "leg_f_bind"
        ),
        "{scenario} A-1': the Failed row must carry \
         MtlsInterceptInstallFailed(stage=leg_f_bind) — the armed leg-F bind refusal travelled \
         through the production guard — got {:?}",
        rows[1].reason,
    );
}

/// S-MIF-04 A-1' — BLOCKED, see the module doc.
///
/// Reproduced failure: the drained write-order universe holds ONE row, the
/// `Running` row (`LogicalTimestamp { counter: 1, writer: NodeId("node-001") }`).
/// The superseding `Failed` row carries a byte-identical timestamp, loses the
/// LWW merge in `apply_alloc_status`, and is dropped before it can fan out.
#[tokio::test]
#[ignore = "blocked on the fail-closed LogicalTimestamp collision — fail_closed_on_mtls_install \
            stamps its Failed row from the same tick + node_id as the Running row it supersedes, \
            so dominates() is false and the row is dropped by both SimObservationStore and the \
            production overdrive-store-local LWW. Un-ignore with the fix; see the module doc."]
async fn start_allocation_install_failure_supersedes_running_with_failed() {
    if !is_root() {
        eprintln!(
            "SKIP start_allocation_install_failure_supersedes_running_with_failed: not root"
        );
        return;
    }
    eprintln!("EXECUTED S-MIF-04 A-1' (root)");

    let outcome = drive_fail_closed(Arm::Start, 2, "mif-start-a1").await;

    assert_supersession_observable("S-MIF-04", &outcome);
}

/// S-MIF-05 A-1' — BLOCKED on the same defect as its `StartAllocation` sibling.
#[tokio::test]
#[ignore = "blocked on the fail-closed LogicalTimestamp collision — see the sibling \
            start_allocation_install_failure_supersedes_running_with_failed and the module doc."]
async fn restart_allocation_install_failure_supersedes_running_with_failed() {
    if !is_root() {
        eprintln!(
            "SKIP restart_allocation_install_failure_supersedes_running_with_failed: not root"
        );
        return;
    }
    eprintln!("EXECUTED S-MIF-05 A-1' (root)");

    let outcome = drive_fail_closed(Arm::Restart, 3, "mif-restart-a1").await;

    assert_supersession_observable("S-MIF-05", &outcome);
}
