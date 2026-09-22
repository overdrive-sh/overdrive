//! Tier-3 acceptance for the transparent-mTLS intercept-install FAIL-CLOSED
//! CALL-SITE ORDERING (GH #250 / ADR-0076 § 5.2; DISTILL S-MIF-04 `@keystone`
//! + S-MIF-05).
//!
//! Drives the PRODUCTION driving port `action_shim::dispatch` with a
//! `SimMtlsIntercept` armed to refuse the leg-F bind, on BOTH arms that carry
//! the fail-closed guard — `StartAllocation` (`mod.rs:1294-1308`) and
//! `RestartAllocation` (`:1494-1508`).
//!
//! The same file also drives predecessor-to-fresh-successor restart failure
//! boundaries with a deterministic network adapter: successor failure unwinds
//! only successor resources before exact-old cleanup, while predecessor stop
//! failure retains both distinct owners. These cases share the exact
//! worker/action-shim ordering seam this file already owns and require no real
//! netns.
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
//! # The four assertions
//!
//! The design pins four observables per arm (§ 5.2 / OQ-7); all four are live:
//!
//! | | Assertion | Home |
//! |---|---|---|
//! | A-6' | `release_for_exit_emission` NEVER called | live, below |
//! | A-8' | `driver.on_alloc_running` never called | live, below |
//! | A-9' | the alloc's structural network owner is removed | live, below |
//! | A-1' | `Running` written FIRST, then superseded by `Failed` | live, below |
//!
//! # A-1' found a real production defect, which is now FIXED
//!
//! A-1' is the first test in the codebase able to observe the fail-closed path
//! through the real `action_shim::dispatch`, and on its first execution it
//! reproduced a genuine production defect. It was:
//!
//! > `fail_closed_on_mtls_install` built its superseding `Failed` row from the
//! > SAME `tick` and the SAME `node_id` as the `Running` row it had to
//! > supersede, both resolving through the same tick-derived helper, so both
//! > rows carried a BYTE-IDENTICAL `(counter = tick.tick + 1, writer =
//! > node_id)`. `LogicalTimestamp::dominates` returns `false` on an equal
//! > counter with an equal writer, so the `Failed` row LOST the LWW merge and
//! > was silently dropped — by `SimObservationStore::apply_alloc_status` AND by
//! > the production single-node `overdrive-store-local::apply_alloc_status_lww`
//! > that `run_server` wires via `wire_single_node_observation`.
//!
//! The operator-visible consequence under a real `serve` + `deploy` was that an
//! mTLS intercept-install failure left the allocation **durably recorded
//! `Running` with no interception installed**. The driver WAS stopped and the
//! `LifecycleEvent` WAS emitted, so the workload was never left running
//! uninstrumented — but the durable record lied, which is the surface this
//! feature exists to defend.
//!
//! The fix of record is ADR-0076 rev 5 § Decision 7 and
//! `docs/feature/mtls-intercept-install-fault-seam/design/architecture.md`
//! § 4.8: a same-tick supersede derives its LWW counter from the row it
//! supersedes, never from the tick, and `build_alloc_status_row`'s stamp became
//! a REQUIRED parameter so every writer decides it explicitly.
//! `LogicalTimestamp::dominates` was NOT changed — the comparator is correct;
//! the counter the shim assigned was wrong.
//!
//! **ADR-0077 generalised that fix to EVERY durable write site.** The
//! prior-derivation mechanism now lives in one constructor,
//! `LogicalTimestamp::dominating(tick_floor, writer, prior)`, and the two
//! bespoke shim helpers (`timestamp_for` / `superseding_timestamp`) were
//! deleted. The same-tick supersede this file defends is now the `prior =
//! Some(&running_row.updated_at)` call at the `fail_closed_on_mtls_install`
//! site; the behaviour asserted below is unchanged.
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
//!                                          structural network torn down
//! ```
//!
//! The absent netns/slot on the last edge is what A-9' characterises: install
//! failure attempts the driver, partial-mTLS, and structural-network cleanup
//! before recording the superseding Failed disposition.
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
//! `ovd-ns-<slot>`) so its real-kernel names do not overlap another scenario.
//! The integration module still joins the `host-kernel-shared` nextest group:
//! restart adoption owns a process-global `ovd-ns-*` GC pass. Run via
//! `cargo xtask lima run -- cargo nextest run
//! -p overdrive-control-plane --features integration-tests`. NEVER `--no-run`.
//!
//! Cleanup: a `NetnsGuard` RAII teardown plus an explicit pre-sweep at each use
//! site, mirroring `alloc_netns_lifecycle.rs` — this repo has a documented
//! cross-run leak-hazard class for exactly this shape.

#![cfg(target_os = "linux")]
// Skip-on-no-privilege and executed-marker messages are the legitimate way
// these Tier-3 tests communicate their lane to the test log.
#![allow(clippy::print_stderr)]
#![allow(
    clippy::large_futures,
    reason = "GH #295 exact source-honest shared-network errors increase the existing composed dispatch future until the single cut lands"
)]
// A-1'/A-6'/A-8'/A-9' etc. read as prose labels in the scenario docs, not code.
#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex as StdMutex};
use std::time::{Duration, Instant};

use futures::StreamExt;
use tokio::sync::broadcast;

use overdrive_control_plane::action_shim::{
    MtlsInterceptLifecycle, ShimError, WorkloadNetworkProvisioner, dispatch,
    dispatch_with_network_provisioner,
};
use overdrive_control_plane::veth_provisioner::{
    NetSlot, NetSlotAllocator, VethProvisionError, VmTapPlan, WorkloadNetnsPlan,
    derive_workload_netns_plan, responder_addr_for_slot, teardown_workload_netns,
};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, CertSerial, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverStartClass,
    DriverStartFailure, DriverType, Resources,
};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LagAwareSubscription, LogicalTimestamp, ObservationRow,
    ObservationStore, ObservationStoreError, SubscriptionEvent,
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
use overdrive_worker::mtls_intercept::InterceptPostcondition;
use overdrive_worker::mtls_intercept_port::{InterceptGuard, MtlsIntercept};
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
    Arc::new(MtlsInterceptWorker::new(enforcement, resolve, Arc::new(SimClock::new()), intercept))
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
        driver: overdrive_core::traits::driver::DriverPayload::Vm(
            overdrive_core::traits::driver::VmPayload {
                command: "/bin/true".to_owned(),
                args: Vec::new(),
                kernel: PathBuf::from("/nonexistent/kernel"),
                rootfs: PathBuf::from("/nonexistent/rootfs"),
            },
        ),
        resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        // The C3 provision seam SETS these — supplied `None` so the seam's own
        // assign/provision/inject runs for real.
        network: None,
        service_ports: Vec::new(),
    }
}

/// Held identity fixture for paths whose subject was already issued before
/// dispatch. It prevents the SVID-audit write from consuming an injected
/// lifecycle-write refusal, keeping the fault pinned to `Running`.
fn held_svid(workload: &WorkloadId, alloc: &AllocationId) -> SvidMaterial {
    SvidMaterial::new(
        CaCertPem::new("-----BEGIN CERTIFICATE-----\nLEAF\n-----END CERTIFICATE-----\n".into()),
        CaCertDer::new(vec![0xDE, 0xAD]),
        CertSerial::new("0badc0de").expect("serial parses"),
        SpiffeId::for_allocation(workload, alloc),
        CaKeyPem::new("-----BEGIN PRIVATE KEY-----\nKEY\n-----END PRIVATE KEY-----\n".into()),
        UnixInstant::from_unix_duration(Duration::from_secs(1_700_003_600)),
    )
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
/// `slot`. Each test owns a DISTINCT slot drawn from this file's registry band,
/// so its `ovd-ns-<slot>` name does not overlap another scenario.
fn arm_netns_guard(slot: NetSlot) -> NetnsGuard {
    let plan = derive_workload_netns_plan(slot, responder_addr_for_slot(slot));
    let _ = teardown_workload_netns(&plan);
    NetnsGuard { plan }
}

/// Reserve every lower slot so the allocation under test receives its
/// registered test slot rather than the otherwise-smallest-free slot zero.
fn allocator_pinned_to_slot(alloc: &AllocationId, slot: NetSlot) -> NetSlotAllocator {
    let allocator = NetSlotAllocator::new();
    let slot_number: u16 = slot.to_string().parse().expect("NetSlot Display is canonical u16");
    for reserved in 0..slot_number {
        let holder = AllocationId::new(&format!("mtls-slot-reservation-{reserved}"))
            .expect("valid holder id");
        allocator
            .adopt(holder, NetSlot::new(reserved).expect("reserved slot is in range"))
            .expect("each reservation owns a distinct slot");
    }
    allocator.adopt(alloc.clone(), slot).expect("adopt this file's band slot");
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
    starts: parking_lot::Mutex<Vec<AllocationId>>,
    releases: parking_lot::Mutex<Vec<AllocationId>>,
    on_alloc_running_calls: parking_lot::Mutex<Vec<AllocationId>>,
}

impl RecordingDriver {
    fn new() -> Self {
        Self {
            inner: SimDriver::new(DriverType::Vm),
            starts: parking_lot::Mutex::new(Vec::new()),
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
        self.starts.lock().push(spec.alloc.clone());
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

    async fn release_for_exit_emission(&self, handle: &AllocationHandle) {
        self.releases.lock().push(handle.alloc.clone());
        self.inner.release_for_exit_emission(handle).await;
    }

    fn on_alloc_running(&self, spec: &AllocationSpec) {
        self.on_alloc_running_calls.lock().push(spec.alloc.clone());
        self.inner.on_alloc_running(spec);
    }
}

/// Driver double whose async release is deliberately held. Its Drop marker
/// makes cancellation observable without starting a second task inside the
/// method—the exact structured-concurrency property under test.
struct HoldingReleaseDriver {
    inner: SimDriver,
    release_entered: tokio::sync::Semaphore,
    release_permit: tokio::sync::Semaphore,
    release_cancelled: AtomicBool,
    release_completed: AtomicBool,
    on_alloc_running_called: AtomicBool,
}

impl HoldingReleaseDriver {
    fn new() -> Self {
        Self {
            inner: SimDriver::new(DriverType::Vm),
            release_entered: tokio::sync::Semaphore::new(0),
            release_permit: tokio::sync::Semaphore::new(0),
            release_cancelled: AtomicBool::new(false),
            release_completed: AtomicBool::new(false),
            on_alloc_running_called: AtomicBool::new(false),
        }
    }
}

struct HeldReleaseDrop<'a> {
    cancelled: &'a AtomicBool,
    completed: bool,
}

impl Drop for HeldReleaseDrop<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.cancelled.store(true, Ordering::SeqCst);
        }
    }
}

#[async_trait::async_trait]
impl Driver for HoldingReleaseDriver {
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

    async fn release_for_exit_emission(&self, handle: &AllocationHandle) {
        let mut drop_marker =
            HeldReleaseDrop { cancelled: &self.release_cancelled, completed: false };
        self.release_entered.add_permits(1);
        self.release_permit.acquire().await.expect("release permit remains open").forget();
        self.inner.release_for_exit_emission(handle).await;
        self.release_completed.store(true, Ordering::SeqCst);
        drop_marker.completed = true;
    }

    fn on_alloc_running(&self, spec: &AllocationSpec) {
        assert!(
            self.release_completed.load(Ordering::SeqCst),
            "on_alloc_running must follow completed async release for {}",
            spec.alloc
        );
        self.on_alloc_running_called.store(true, Ordering::SeqCst);
    }
}

/// Drive a single `Action` through the production `action_shim::dispatch` with
/// the supplied `driver` + `net_slot_allocator` + a REAL `MtlsInterceptWorker`
/// (so the C3 provision seam AND the mTLS install seam are both ARMED). Every
/// orthogonal port is a sim double.
async fn dispatch_one(
    action: Action,
    drivers: &overdrive_core::traits::driver::DriverRegistry,
    alloc_drivers: &overdrive_control_plane::action_shim::AllocDriverIndex,
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
        drivers,
        alloc_drivers,
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
        &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
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
    /// D31 — allocations whose driver start boundary was entered.
    starts: Vec<AllocationId>,
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

/// Seed an eligible terminal predecessor so the `RestartAllocation` arm's
/// `find_prior_alloc_row` resolves `(workload_id, node_id)`.
///
/// `counter: 0` supplies a stable accepted predecessor timestamp; successor
/// writes use a distinct key and never dominate or overwrite it.
async fn seed_restart_predecessor(
    obs: &dyn ObservationStore,
    alloc: &AllocationId,
    workload: &WorkloadId,
    node: &NodeId,
) {
    let row = AllocStatusRow {
        alloc_id: alloc.clone(),
        workload_id: workload.clone(),
        node_id: node.clone(),
        state: AllocState::Failed,
        updated_at: LogicalTimestamp { counter: 0, writer: node.clone() },
        reason: Some(TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(17),
            signal: None,
            stderr_tail: None,
        }),
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    };
    obs.write_alloc_lifecycle(
        row,
        overdrive_core::traits::observation_store::TransitionSource::Reconciler,
    )
    .await
    .expect("seed prior Running alloc row");
}

/// Drive `arm` through the real `dispatch` with the leg-F bind refused, on a
/// dedicated `slot` (this file's registry band), and collect every port-exposed
/// observable.
///
/// The `Arc::clone` BEFORE the cast is load-bearing: the test retains a typed
/// `Arc<SimMtlsIntercept>` so it can arm (and if needed `clear_faults()`) after
/// the worker holds its `Arc<dyn MtlsIntercept>`.
async fn drive_fail_closed(arm: Arm, slot: NetSlot, alloc_name: &str) -> FailClosedOutcome {
    let tmp = TempDir::new().expect("tempdir");
    let store: Arc<dyn overdrive_core::traits::intent_store::IntentStore> =
        Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open store"));
    let obs = build_obs();

    let intercept = Arc::new(SimMtlsIntercept::new());
    intercept.script_bind_fault(SimInterceptFault::TransparentListener { errno: libc::EPERM });
    let worker = build_worker(Arc::clone(&intercept) as Arc<dyn MtlsIntercept>);

    let driver = Arc::new(RecordingDriver::new());
    let drivers: Arc<overdrive_core::traits::driver::DriverRegistry> = {
        let mut r = overdrive_core::traits::driver::DriverRegistry::new();
        r.insert(Arc::clone(&driver) as Arc<dyn Driver>);
        Arc::new(r)
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();

    let alloc = AllocationId::new(alloc_name).expect("valid alloc id");
    let predecessor = AllocationId::new(&format!("{alloc_name}-predecessor"))
        .expect("valid predecessor alloc id");
    let successor =
        AllocationId::new(&format!("{alloc_name}-successor")).expect("valid successor alloc id");
    let workload = WorkloadId::new(&format!("svc-{alloc_name}")).expect("valid workload id");
    let node = NodeId::new("node-001").expect("valid node id");

    // TEST ISOLATION (cross-file net-slot convention): pin THIS test's alloc to
    // its file's DISTINCT registry-band slot via the production `adopt` seam so
    // its system-global `ovd-ns-<slot>` / veth / `/30` names never collide with
    // a sibling arm OR any other file's netns test under nextest's
    // process-per-test parallelism. dispatch's internal
    // `provision_and_inject_netns` → `assign(alloc)` returns this pre-adopted
    // slot idempotently. For Restart the pinned allocation is the fresh
    // successor; the terminal predecessor owns no structural fixture here.
    let effect_alloc = if matches!(arm, Arm::Start) { &alloc } else { &successor };
    let allocator = allocator_pinned_to_slot(effect_alloc, slot);
    let _guard = arm_netns_guard(slot);

    let action = match arm {
        Arm::Start => Action::StartAllocation {
            alloc_id: alloc.clone(),
            workload_id: workload.clone(),
            node_id: node.clone(),
            spec: build_spec(&alloc),
            kind: WorkloadKind::Service,
        },
        Arm::Restart => {
            // The restart arm resolves stable facts from an eligible terminal
            // predecessor. Seeded before subscription so only fresh-successor
            // writes enter the asserted write-order universe.
            seed_restart_predecessor(obs.as_ref(), &predecessor, &workload, &node).await;
            Action::RestartAllocation {
                alloc_id: predecessor.clone(),
                spec: build_spec(&successor),
                kind: WorkloadKind::Service,
            }
        }
    };

    // Opened BEFORE the dispatch — the write-order universe is only observable
    // on a subscription that predates the writes.
    let mut subscription: LagAwareSubscription =
        obs.subscribe_all_events().await.expect("subscribe to the observation store");

    dispatch_one(
        action,
        drivers.as_ref(),
        &alloc_drivers,
        obs.as_ref(),
        Arc::clone(&store),
        &worker,
        &allocator,
    )
    .await
    .expect(
        "the install failure must be RECORDED and the dispatch return Ok — a bubbled Err is \
             the indefinite-Pending-retry regression",
    );

    let rows = drain_alloc_rows(&mut subscription, effect_alloc).await;
    let outcome = FailClosedOutcome {
        rows,
        starts: driver.starts.lock().clone(),
        releases: driver.releases.lock().clone(),
        on_alloc_running_calls: driver.on_alloc_running_calls.lock().clone(),
        slot_still_held: allocator.snapshot().contains_key(effect_alloc),
    };

    worker.stop_alloc(effect_alloc).await.expect("allocation teardown succeeds");
    worker.stop_alloc(&predecessor).await.expect("predecessor teardown succeeds");
    outcome
}

// ---------------------------------------------------------------------------
// A-6' / A-8' / A-9' — the CALL-SITE ORDERING properties. The port's ONE
// justification, and the litmus-proven core of this step.
// ---------------------------------------------------------------------------

/// Accepted post-`Running` ordering: the driver starts exactly once, but the
/// deferred execution release and running hook stay closed when install fails.
///
/// A-6' is a property of the call site's `return` placement — the arm returns
/// BEFORE `driver.release_for_exit_emission(handle)` a few lines below — so a
/// reordering that released first survives the helper-level contract entirely.
/// This assertion, and only this one, dies on that reordering.
fn assert_ordering_observables(scenario: &str, outcome: &FailClosedOutcome) {
    assert_eq!(
        outcome.starts.len(),
        1,
        "{scenario}: the accepted capture-ready -> driver start -> Running -> intercept sequence enters the driver exactly once, got {:?}",
        outcome.starts,
    );
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
    assert!(
        !outcome.slot_still_held,
        "{scenario} A-9': the post-Running failure unwind must remove the structural network owner before recording Failed",
    );
}

// ---------------------------------------------------------------------------
// Rejected initial Running writes. C3 provisioning precedes Driver::start, so
// its structural resources must unwind even though the observation store
// cannot record the initial Running row.
// ---------------------------------------------------------------------------

struct RunningWriteRejectNetwork {
    provisions: AtomicUsize,
    teardowns: AtomicUsize,
}

impl WorkloadNetworkProvisioner for RunningWriteRejectNetwork {
    fn provision(
        &self,
        _workload: &WorkloadNetnsPlan,
        _vm_tap: &VmTapPlan,
    ) -> Result<(), VethProvisionError> {
        self.provisions.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn teardown(&self, _workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        self.teardowns.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct RunningWriteRejectOutcome {
    result: Result<(), ShimError>,
    starts: Vec<AllocationId>,
    provisions: usize,
    teardowns: usize,
    slot_still_held: bool,
}

/// Drive the production start/restart arm through a rejected initial Running
/// write after C3 provision and driver start, using a deterministic structural
/// network adapter rather than requiring host network privileges.
async fn drive_running_write_rejection(arm: Arm) -> RunningWriteRejectOutcome {
    let tmp = TempDir::new().expect("tempdir");
    let store: Arc<dyn overdrive_core::traits::intent_store::IntentStore> =
        Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open store"));
    let obs = build_obs();
    let worker = build_worker(Arc::new(SimMtlsIntercept::new()));
    let driver = Arc::new(RecordingDriver::new());
    let drivers: Arc<overdrive_core::traits::driver::DriverRegistry> = {
        let mut registry = overdrive_core::traits::driver::DriverRegistry::new();
        registry.insert(Arc::clone(&driver) as Arc<dyn Driver>);
        Arc::new(registry)
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    let net_slots = NetSlotAllocator::new();
    let alloc = AllocationId::new(match arm {
        Arm::Start => "running-write-reject-start",
        Arm::Restart => "running-write-reject-restart",
    })
    .expect("valid allocation id");
    let predecessor = AllocationId::new("running-write-reject-restart-predecessor")
        .expect("valid predecessor allocation id");
    let successor = AllocationId::new("running-write-reject-restart-successor")
        .expect("valid successor allocation id");
    let effect_alloc = if matches!(arm, Arm::Start) { &alloc } else { &successor };
    let workload = WorkloadId::new("svc-running-write-reject").expect("valid workload id");
    let node = NodeId::new("node-001").expect("valid node id");
    let identity = overdrive_control_plane::identity_mgr::IdentityMgr::new(None);
    identity.hold(effect_alloc.clone(), held_svid(&workload, effect_alloc));
    let action = match arm {
        Arm::Start => Action::StartAllocation {
            alloc_id: alloc.clone(),
            workload_id: workload,
            node_id: node,
            spec: build_spec(&alloc),
            kind: WorkloadKind::Service,
        },
        Arm::Restart => {
            seed_restart_predecessor(obs.as_ref(), &predecessor, &workload, &node).await;
            Action::RestartAllocation {
                alloc_id: predecessor,
                spec: build_spec(&successor),
                kind: WorkloadKind::Service,
            }
        }
    };
    obs.inject_write_failure(ObservationStoreError::Unreachable {
        peer: "rejected-running-write".to_owned(),
    });
    let network = RunningWriteRejectNetwork {
        provisions: AtomicUsize::new(0),
        teardowns: AtomicUsize::new(0),
    };
    let dataplane = overdrive_sim::adapters::dataplane::SimDataplane::new();
    let ca = overdrive_sim::adapters::ca::SimCa::new(Arc::new(
        overdrive_sim::adapters::entropy::SimEntropy::new(0),
    ));
    let clock = SimClock::new();
    let (lifecycle_tx, _lifecycle_rx) = broadcast::channel(64);
    let writer_node = NodeId::new("writer-1").expect("node id");
    let broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    let mtls_lifecycle = (&worker) as &dyn MtlsInterceptLifecycle;
    let result = dispatch_with_network_provisioner(
        vec![action],
        drivers.as_ref(),
        &alloc_drivers,
        obs.as_ref(),
        &dataplane,
        &ca,
        &clock,
        &identity,
        &lifecycle_tx,
        &tick_now(),
        &writer_node,
        build_vip_allocator(store),
        &broker,
        None,
        Some(mtls_lifecycle),
        &net_slots,
        &network,
        &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
    )
    .await;

    RunningWriteRejectOutcome {
        result,
        starts: driver.starts.lock().clone(),
        provisions: network.provisions.load(Ordering::SeqCst),
        teardowns: network.teardowns.load(Ordering::SeqCst),
        slot_still_held: net_slots.snapshot().contains_key(effect_alloc),
    }
}

fn assert_running_write_rejection_unwinds_network(
    scenario: &str,
    outcome: &RunningWriteRejectOutcome,
) {
    assert!(
        matches!(&outcome.result, Err(ShimError::Observation(_))),
        "{scenario}: the injected initial Running write rejection must surface; got {:?}",
        outcome.result
    );
    assert_eq!(outcome.starts.len(), 1, "{scenario}: driver must start before the rejected write");
    assert_eq!(outcome.provisions, 1, "{scenario}: C3 must provision exactly one network owner");
    assert_eq!(outcome.teardowns, 1, "{scenario}: rejected write must tear that owner down");
    assert!(
        !outcome.slot_still_held,
        "{scenario}: structural teardown must release the allocation's slot"
    );
}

/// A rejected fresh-start Running write unwinds its C3 network owner.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn start_running_write_rejection_tears_down_network_and_releases_slot() {
    let outcome = drive_running_write_rejection(Arm::Start).await;
    assert_running_write_rejection_unwinds_network("fresh start", &outcome);
}

/// A rejected restarted Running write follows the same structural unwind.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn restart_running_write_rejection_tears_down_network_and_releases_slot() {
    let outcome = drive_running_write_rejection(Arm::Restart).await;
    assert_running_write_rejection_unwinds_network("restart", &outcome);
}

/// S-MIF-04 (`@keystone`) — a failed intercept install on a FRESH allocation
/// keeps its exit watcher. Defends the `StartAllocation` guard's
/// `return`-before-release placement (`mod.rs:1297-1307` before `:1309`).
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
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

    let outcome = drive_fail_closed(
        Arm::Start,
        super::net_slots::MTLS_INSTALL_FAIL_CLOSED.nth(0),
        "mif-start",
    )
    .await;

    assert_ordering_observables("S-MIF-04", &outcome);
}

/// S-MIF-05 — a failed intercept install on a RESTARTED allocation keeps its
/// exit watcher. Defends the `RestartAllocation` guard's
/// `return`-before-release placement (`mod.rs:1497-1507` before `:1509`).
///
/// The two production blocks are byte-identical TODAY, so what this adds over
/// S-MIF-04 is a defense against a FUTURE DIVERGENT EDIT to one of them — the
/// real risk, and why the two are not collapsed.
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
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

    let outcome = drive_fail_closed(
        Arm::Restart,
        super::net_slots::MTLS_INSTALL_FAIL_CLOSED.nth(1),
        "mif-restart",
    )
    .await;

    assert_ordering_observables("S-MIF-05", &outcome);
}

// ---------------------------------------------------------------------------
// A-1' — the ROW SUPERSESSION property. This pair reproduced the LWW collision
// described in the module doc; both are live against the fix.
// ---------------------------------------------------------------------------

/// A-1' — the permitted transient `Running` row is immediately superseded by
/// a dominating `Failed` row carrying
/// `MtlsInterceptInstallFailed { stage: "leg_f_bind", .. }`.
///
/// Proves `start_alloc`'s `Err` reaches the helper THROUGH the production
/// guard, which no helper-level test can establish.
fn assert_supersession_observable(scenario: &str, outcome: &FailClosedOutcome) {
    let rows = &outcome.rows;
    assert_eq!(
        rows.len(),
        2,
        "{scenario} A-1': the dispatch must write Running then its superseding Failed row — got {rows:?}",
    );
    assert_eq!(
        rows[0].state,
        AllocState::Running,
        "{scenario} A-1': driver readiness is recorded before install, got {:?} ({:?})",
        rows[0].state,
        rows[0].reason,
    );
    assert_eq!(
        rows[1].state,
        AllocState::Failed,
        "{scenario} A-1': the final row must be Failed, got {:?} ({:?})",
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
    assert!(
        rows[1].updated_at.dominates(&rows[0].updated_at),
        "{scenario} A-1': Failed must strictly dominate transient Running: {rows:?}",
    );
    assert_eq!(
        rows[1].started_at, rows[0].started_at,
        "{scenario} A-1': supersession retains the truthful Running timestamp",
    );
}

/// S-MIF-04 A-1' — the `StartAllocation` arm's supersession.
///
/// This is the test that reproduced the LWW collision: before the fix the
/// drained write-order universe held ONE row, the `Running` row
/// (`LogicalTimestamp { counter: 1, writer: NodeId("node-001") }`), because the
/// superseding `Failed` row carried a byte-identical timestamp, lost the merge
/// in `apply_alloc_status`, and was dropped before it could fan out.
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn start_allocation_install_failure_supersedes_running_with_failed() {
    if !is_root() {
        eprintln!("SKIP start_allocation_install_failure_supersedes_running_with_failed: not root");
        return;
    }
    eprintln!("EXECUTED S-MIF-04 A-1' (root)");

    let outcome = drive_fail_closed(
        Arm::Start,
        super::net_slots::MTLS_INSTALL_FAIL_CLOSED.nth(2),
        "mif-start-a1",
    )
    .await;

    assert_supersession_observable("S-MIF-04", &outcome);
}

/// S-MIF-05 A-1' — the `RestartAllocation` arm's supersession, which reproduced
/// the same collision as its `StartAllocation` sibling through the same shared
/// helper.
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn restart_allocation_install_failure_supersedes_running_with_failed() {
    if !is_root() {
        eprintln!(
            "SKIP restart_allocation_install_failure_supersedes_running_with_failed: not root"
        );
        return;
    }
    eprintln!("EXECUTED S-MIF-05 A-1' (root)");

    let outcome = drive_fail_closed(
        Arm::Restart,
        super::net_slots::MTLS_INSTALL_FAIL_CLOSED.nth(3),
        "mif-restart-a1",
    )
    .await;

    assert_supersession_observable("S-MIF-05", &outcome);
}

/// The production StartAllocation arm must await the existing async Driver
/// release hook after a successful intercept install. Cancelling dispatch
/// while that hook is held must drop the same owned future; it may not proceed
/// to `on_alloc_running` or leave a detached release behind.
///
/// Observable universe: dispatch completion plus every boolean exposed by
/// `HoldingReleaseDriver`. The permitted cancellation delta is exactly
/// release-entered=false->true and release-cancelled=false->true; release
/// completion and the subsequent lifecycle hook remain false.
///
/// CONTRACT_SHAPE: bounded-change.
/// Outcome anchor: DISCUSS Elevator Pitch
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test]
async fn start_allocation_awaits_release_and_cancellation_owns_the_future() {
    if !is_root() {
        eprintln!(
            "SKIP start_allocation_awaits_release_and_cancellation_owns_the_future: not root"
        );
        return;
    }

    let tmp = TempDir::new().expect("tempdir");
    let store: Arc<dyn overdrive_core::traits::intent_store::IntentStore> =
        Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open store"));
    let obs = build_obs();
    let worker = build_worker(Arc::new(SimMtlsIntercept::new()));
    let driver = Arc::new(HoldingReleaseDriver::new());
    let drivers: Arc<overdrive_core::traits::driver::DriverRegistry> = {
        let mut registry = overdrive_core::traits::driver::DriverRegistry::new();
        registry.insert(Arc::clone(&driver) as Arc<dyn Driver>);
        Arc::new(registry)
    };
    let alloc_drivers = Arc::new(overdrive_control_plane::action_shim::AllocDriverIndex::default());
    let allocator = Arc::new(NetSlotAllocator::new());
    let alloc = AllocationId::new("gti-held-release").expect("valid alloc id");
    let workload = WorkloadId::new("svc-gti-held-release").expect("valid workload id");
    let node = NodeId::new("node-001").expect("valid node id");
    let slot = super::net_slots::MTLS_INSTALL_FAIL_CLOSED.nth(4);
    allocator.adopt(alloc.clone(), slot).expect("adopt this file's band slot");
    let _guard = arm_netns_guard(slot);
    let action = Action::StartAllocation {
        alloc_id: alloc.clone(),
        workload_id: workload,
        node_id: node,
        spec: build_spec(&alloc),
        kind: WorkloadKind::Service,
    };

    let task = {
        let drivers = Arc::clone(&drivers);
        let alloc_drivers = Arc::clone(&alloc_drivers);
        let obs = Arc::clone(&obs);
        let store = Arc::clone(&store);
        let worker = Arc::clone(&worker);
        let allocator = Arc::clone(&allocator);
        tokio::spawn(async move {
            dispatch_one(
                action,
                drivers.as_ref(),
                alloc_drivers.as_ref(),
                obs.as_ref(),
                store,
                &worker,
                allocator.as_ref(),
            )
            .await
        })
    };

    tokio::time::timeout(Duration::from_secs(10), driver.release_entered.acquire())
        .await
        .expect("dispatch reaches the post-install async release")
        .expect("release-entered semaphore remains open")
        .forget();
    assert!(!task.is_finished(), "dispatch must remain pending inside the held release future");
    assert!(!driver.release_completed.load(Ordering::SeqCst));
    assert!(!driver.on_alloc_running_called.load(Ordering::SeqCst));

    task.abort();
    assert!(task.await.expect_err("dispatch task is cancelled").is_cancelled());
    tokio::task::yield_now().await;
    assert!(
        driver.release_cancelled.load(Ordering::SeqCst),
        "cancelling dispatch must drop the same release future"
    );
    assert!(!driver.release_completed.load(Ordering::SeqCst));
    assert!(!driver.on_alloc_running_called.load(Ordering::SeqCst));

    worker.stop_alloc(&alloc).await.expect("allocation teardown succeeds");
    let _ = driver.stop(&AllocationHandle { alloc, pid: None }).await;
}

// ---------------------------------------------------------------------------
// Fresh-successor restart abort cleanup. The network adapter asserts exact-old
// mTLS-before-network ordering at its driven port boundary; successor failure
// cuts retain their existing fail-closed cleanup behavior.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RestartAbortScenario {
    Provision,
    Identity,
    DriverStart,
    DriverStop,
}

struct RestartAbortDriver {
    scenario: RestartAbortScenario,
    starts: parking_lot::Mutex<Vec<AllocationId>>,
    stops: parking_lot::Mutex<Vec<AllocationId>>,
}

#[async_trait::async_trait]
impl Driver for RestartAbortDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Vm
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        self.starts.lock().push(spec.alloc.clone());
        if self.scenario == RestartAbortScenario::DriverStart {
            return Err(DriverError::StartRejected {
                failure: DriverStartFailure {
                    class: DriverStartClass::Unclassified { driver: DriverType::Vm },
                    detail: "injected restart driver-start rejection".to_owned(),
                },
            });
        }
        Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: Some(4242) })
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        self.stops.lock().push(handle.alloc.clone());
        if self.scenario == RestartAbortScenario::DriverStop {
            return Err(DriverError::Io(std::io::Error::other(
                "injected prior-driver stop failure",
            )));
        }
        Ok(())
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        Err(DriverError::NotFound { alloc: handle.alloc.clone() })
    }

    async fn resize(
        &self,
        _handle: &AllocationHandle,
        _resources: Resources,
    ) -> Result<(), DriverError> {
        Ok(())
    }
}

struct RestartAbortNetwork {
    scenario: RestartAbortScenario,
    worker: Arc<MtlsInterceptWorker>,
    predecessor: AllocationId,
    predecessor_netns: String,
    provisions: AtomicUsize,
    teardowns: AtomicUsize,
    teardown_observed_intercept_stopped: AtomicBool,
}

impl WorkloadNetworkProvisioner for RestartAbortNetwork {
    fn provision(
        &self,
        _workload: &WorkloadNetnsPlan,
        _vm_tap: &VmTapPlan,
    ) -> Result<(), VethProvisionError> {
        self.provisions.fetch_add(1, Ordering::SeqCst);
        if self.scenario == RestartAbortScenario::Provision {
            return Err(VethProvisionError::SysctlSetFailed {
                key: "net.ipv4.ip_forward".to_owned(),
                value: "1".to_owned(),
                path: "/injected/restart/provision".to_owned(),
                source: std::io::Error::other("injected restart provision failure"),
            });
        }
        Ok(())
    }

    fn teardown(&self, workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        self.teardowns.fetch_add(1, Ordering::SeqCst);
        if workload.netns.as_str() == self.predecessor_netns {
            let stopped = self.worker.leg_c_addr(&self.predecessor).is_none()
                && self.worker.alloc_stop_converged_for_test(&self.predecessor);
            self.teardown_observed_intercept_stopped.store(stopped, Ordering::SeqCst);
            assert!(
                stopped,
                "exact-old structural teardown requires predecessor interception teardown"
            );
        }
        Ok(())
    }
}

struct RestartAbortOutcome {
    result: Result<(), ShimError>,
    row: Option<AllocStatusRow>,
    predecessor_row: AllocStatusRow,
    predecessor_after: Option<AllocStatusRow>,
    predecessor: AllocationId,
    successor: AllocationId,
    starts: Vec<AllocationId>,
    stops: Vec<AllocationId>,
    provisions: usize,
    teardowns: usize,
    teardown_observed_intercept_stopped: bool,
    prior_intercept: PriorInterceptState,
    stop_alloc_calls: u64,
    predecessor_slot_held: bool,
    successor_slot_held: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PriorInterceptState {
    Live,
    StopConverged,
}

#[allow(
    clippy::too_many_lines,
    reason = "the shared four-partition fixture records both fresh-successor and exact-old outcomes"
)]
async fn drive_restart_abort(scenario: RestartAbortScenario) -> RestartAbortOutcome {
    let tmp = TempDir::new().expect("tempdir");
    let store: Arc<dyn overdrive_core::traits::intent_store::IntentStore> =
        Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open store"));
    let obs = build_obs();
    let worker = build_worker(Arc::new(SimMtlsIntercept::new()));
    let stem = format!("restart-abort-{scenario:?}").to_ascii_lowercase();
    let predecessor = AllocationId::new(&format!("{stem}-0")).expect("valid predecessor alloc id");
    let successor = AllocationId::new(&format!("{stem}-1")).expect("valid successor alloc id");
    let workload = WorkloadId::new("svc-restart-abort").expect("valid workload id");
    let node = NodeId::new("node-001").expect("valid node id");
    let prior_spec = build_spec(&predecessor);
    worker.start_alloc(&prior_spec).await.expect("prior interception installs");
    assert!(worker.leg_c_addr(&predecessor).is_some(), "fixture owns a prior interception");
    seed_restart_predecessor(obs.as_ref(), &predecessor, &workload, &node).await;
    let predecessor_row = obs
        .alloc_status_row(&predecessor)
        .await
        .expect("predecessor row read")
        .expect("eligible terminal predecessor row");
    if scenario == RestartAbortScenario::Identity {
        obs.inject_write_failure(ObservationStoreError::Io(std::io::Error::other(
            "injected SVID audit failure",
        )));
    }

    let driver = Arc::new(RestartAbortDriver {
        scenario,
        starts: parking_lot::Mutex::new(Vec::new()),
        stops: parking_lot::Mutex::new(Vec::new()),
    });
    let drivers = {
        let mut registry = overdrive_core::traits::driver::DriverRegistry::new();
        registry.insert(Arc::clone(&driver) as Arc<dyn Driver>);
        registry
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    let net_slots = NetSlotAllocator::new();
    let predecessor_slot =
        net_slots.assign(predecessor.clone()).expect("restart predecessor owns a network slot");
    let predecessor_plan =
        derive_workload_netns_plan(predecessor_slot, responder_addr_for_slot(predecessor_slot));
    let network = RestartAbortNetwork {
        scenario,
        worker: Arc::clone(&worker),
        predecessor: predecessor.clone(),
        predecessor_netns: predecessor_plan.netns.as_str().to_owned(),
        provisions: AtomicUsize::new(0),
        teardowns: AtomicUsize::new(0),
        teardown_observed_intercept_stopped: AtomicBool::new(false),
    };
    let dataplane = overdrive_sim::adapters::dataplane::SimDataplane::new();
    let ca = overdrive_sim::adapters::ca::SimCa::new(Arc::new(
        overdrive_sim::adapters::entropy::SimEntropy::new(0),
    ));
    let clock = SimClock::new();
    let identity = overdrive_control_plane::identity_mgr::IdentityMgr::new(None);
    let (lifecycle_tx, _lifecycle_rx) = broadcast::channel(64);
    let writer_node = NodeId::new("writer-1").expect("node id");
    let broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    let mtls_lifecycle = (&worker) as &dyn MtlsInterceptLifecycle;
    let result = dispatch_with_network_provisioner(
        vec![Action::RestartAllocation {
            alloc_id: predecessor.clone(),
            spec: build_spec(&successor),
            kind: WorkloadKind::Service,
        }],
        &drivers,
        &alloc_drivers,
        obs.as_ref(),
        &dataplane,
        &ca,
        &clock,
        &identity,
        &lifecycle_tx,
        &tick_now(),
        &writer_node,
        build_vip_allocator(store),
        &broker,
        None,
        Some(mtls_lifecycle),
        &net_slots,
        &network,
        &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
    )
    .await;

    let prior_intercept = match (
        worker.leg_c_addr(&predecessor).is_some(),
        worker.alloc_stop_converged_for_test(&predecessor),
    ) {
        (true, false) => PriorInterceptState::Live,
        (false, true) => PriorInterceptState::StopConverged,
        state => panic!("restart abort left an invalid prior-intercept state: {state:?}"),
    };
    let outcome = RestartAbortOutcome {
        result,
        row: obs.alloc_status_row(&successor).await.expect("successor row read succeeds"),
        predecessor_after: obs
            .alloc_status_row(&predecessor)
            .await
            .expect("predecessor row re-read succeeds"),
        predecessor_row,
        predecessor: predecessor.clone(),
        successor: successor.clone(),
        starts: driver.starts.lock().clone(),
        stops: driver.stops.lock().clone(),
        provisions: network.provisions.load(Ordering::SeqCst),
        teardowns: network.teardowns.load(Ordering::SeqCst),
        teardown_observed_intercept_stopped: network
            .teardown_observed_intercept_stopped
            .load(Ordering::SeqCst),
        prior_intercept,
        stop_alloc_calls: worker.stop_alloc_calls_for_test(),
        predecessor_slot_held: net_slots.snapshot().contains_key(&predecessor),
        successor_slot_held: net_slots.snapshot().contains_key(&successor),
    };
    worker.stop_alloc(&successor).await.expect("successor fixture cleanup converges");
    worker.stop_alloc(&predecessor).await.expect("predecessor fixture cleanup converges");
    outcome
}

fn assert_restart_abort_identities(outcome: &RestartAbortOutcome) {
    assert_eq!(outcome.predecessor_after.as_ref(), Some(&outcome.predecessor_row));
    assert_ne!(outcome.predecessor, outcome.successor);
}

/// Provision failure unwinds the fresh successor structural owner before
/// exact-old predecessor cleanup, releasing both slots.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn restart_provision_failure_tears_down_old_and_replacement_networks_and_releases_the_slot() {
    let outcome = drive_restart_abort(RestartAbortScenario::Provision).await;
    assert_restart_abort_identities(&outcome);
    assert!(outcome.result.is_ok());
    assert!(outcome.starts.is_empty());
    assert_eq!(outcome.stops, vec![outcome.predecessor.clone()]);
    assert_eq!(outcome.provisions, 1);
    assert_eq!(
        outcome.teardowns, 2,
        "failed successor provision unwind and exact-old teardown each run once"
    );
    assert!(outcome.teardown_observed_intercept_stopped);
    assert_eq!(outcome.prior_intercept, PriorInterceptState::StopConverged);
    assert_eq!(outcome.stop_alloc_calls, 1);
    assert!(!outcome.predecessor_slot_held);
    assert!(!outcome.successor_slot_held);
    assert!(matches!(
        outcome.row.and_then(|row| row.reason),
        Some(TransitionReason::WorkloadNetnsProvisionFailed { .. })
    ));
}

/// Identity issuance failure preserves its primary typed error after fresh
/// successor unwind, then completes exact-old predecessor cleanup.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn restart_identity_failure_stops_prior_intercept_before_network_release() {
    let outcome = drive_restart_abort(RestartAbortScenario::Identity).await;
    assert_restart_abort_identities(&outcome);
    assert!(matches!(outcome.result, Err(ShimError::IssueSvid(_))));
    assert!(outcome.starts.is_empty());
    assert_eq!(outcome.stops, vec![outcome.predecessor.clone()]);
    assert_eq!(outcome.provisions, 1);
    assert_eq!(outcome.teardowns, 2);
    assert!(outcome.teardown_observed_intercept_stopped);
    assert_eq!(outcome.prior_intercept, PriorInterceptState::StopConverged);
    assert_eq!(outcome.stop_alloc_calls, 1);
    assert!(!outcome.predecessor_slot_held);
    assert!(!outcome.successor_slot_held);
    assert!(outcome.row.is_none());
}

/// Driver-start rejection records a fresh Failed successor after its unwind,
/// then completes exact-old predecessor cleanup.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn restart_driver_start_failure_stops_prior_intercept_before_network_release() {
    let outcome = drive_restart_abort(RestartAbortScenario::DriverStart).await;
    assert_restart_abort_identities(&outcome);
    assert!(outcome.result.is_ok(), "the rejection is durably recorded as Failed");
    assert_eq!(outcome.starts, vec![outcome.successor.clone()]);
    assert_eq!(outcome.stops, vec![outcome.predecessor.clone()]);
    assert_eq!(outcome.provisions, 1);
    assert_eq!(outcome.teardowns, 2);
    assert!(outcome.teardown_observed_intercept_stopped);
    assert_eq!(outcome.prior_intercept, PriorInterceptState::StopConverged);
    assert_eq!(outcome.stop_alloc_calls, 1);
    assert!(!outcome.predecessor_slot_held);
    assert!(!outcome.successor_slot_held);
    assert!(
        outcome
            .row
            .and_then(|row| row.detail)
            .is_some_and(|detail| detail.contains("injected restart driver-start rejection")),
        "the durable failure preserves the primary driver-start rejection",
    );
}

/// A predecessor driver-stop failure occurs only after the fresh successor is
/// accepted Running, and retains both distinct owners for later convergence.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn restart_driver_stop_failure_retains_mtls_and_network_protection() {
    let outcome = drive_restart_abort(RestartAbortScenario::DriverStop).await;
    assert_restart_abort_identities(&outcome);
    assert!(matches!(outcome.result, Err(ShimError::Driver(_))));
    assert_eq!(outcome.starts, vec![outcome.successor.clone()]);
    assert_eq!(outcome.stops, vec![outcome.predecessor.clone()]);
    assert_eq!(outcome.provisions, 1);
    assert_eq!(outcome.teardowns, 0);
    assert_eq!(outcome.prior_intercept, PriorInterceptState::Live);
    assert_eq!(outcome.stop_alloc_calls, 0);
    assert!(outcome.predecessor_slot_held);
    assert!(outcome.successor_slot_held);
    let successor_row = outcome.row.expect("accepted successor Running row survives old failure");
    assert_eq!(successor_row.alloc_id, outcome.successor);
    assert_eq!(successor_row.state, AllocState::Running);
    assert_eq!(successor_row.restart_count, 0);
    assert!(successor_row.last_terminated.is_none());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RegistrationRetiredEffect {
    NetworkProvision,
    DriverStart,
    DriverStop,
    MtlsElementDrop,
    NetworkTeardown,
}

struct RetirementBarrierGuard {
    _inner: Box<dyn InterceptGuard>,
    drops: Arc<AtomicUsize>,
    effects: Arc<parking_lot::Mutex<Vec<RegistrationRetiredEffect>>>,
}

impl InterceptGuard for RetirementBarrierGuard {}

impl Drop for RetirementBarrierGuard {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        self.effects.lock().push(RegistrationRetiredEffect::MtlsElementDrop);
    }
}

struct RetirementBarrierIntercept {
    inner: SimMtlsIntercept,
    entered: AtomicBool,
    release: (StdMutex<bool>, Condvar),
    block_once: AtomicBool,
    drops: Arc<AtomicUsize>,
    effects: Arc<parking_lot::Mutex<Vec<RegistrationRetiredEffect>>>,
}

impl RetirementBarrierIntercept {
    fn new(effects: Arc<parking_lot::Mutex<Vec<RegistrationRetiredEffect>>>) -> Self {
        Self {
            inner: SimMtlsIntercept::new(),
            entered: AtomicBool::new(false),
            release: (StdMutex::new(false), Condvar::new()),
            block_once: AtomicBool::new(true),
            drops: Arc::new(AtomicUsize::new(0)),
            effects,
        }
    }

    async fn wait_entered(&self) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !self.entered.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("real worker reaches the Pending-to-Retired activation barrier");
    }

    fn release(&self) {
        let (lock, wake) = &self.release;
        *lock.lock().expect("release lock") = true;
        wake.notify_all();
    }

    fn wrap(&self, inner: Box<dyn InterceptGuard>) -> Box<dyn InterceptGuard> {
        Box::new(RetirementBarrierGuard {
            _inner: inner,
            drops: Arc::clone(&self.drops),
            effects: Arc::clone(&self.effects),
        })
    }
}

impl MtlsIntercept for RetirementBarrierIntercept {
    fn bind_transparent(
        &self,
        address: SocketAddrV4,
    ) -> overdrive_worker::mtls_intercept::Result<TcpListener> {
        self.inner.bind_transparent(address)
    }

    fn converge_shared(
        &self,
        prior: Option<&InterceptPostcondition>,
        leg_f: SocketAddrV4,
        leg_c: SocketAddrV4,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        self.inner.converge_shared(prior, leg_f, leg_c)
    }

    fn observe_shared(
        &self,
    ) -> overdrive_worker::mtls_intercept::Result<Option<InterceptPostcondition>> {
        self.inner.observe_shared()
    }

    fn install_outbound(
        &self,
        tap: &str,
        leg_f_port: u16,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        self.inner.install_outbound(tap, leg_f_port).map(|guard| self.wrap(guard))
    }

    fn install_inbound(
        &self,
        virt: SocketAddrV4,
        leg_c_port: u16,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        if self.block_once.swap(false, Ordering::SeqCst) {
            self.entered.store(true, Ordering::SeqCst);
            let (lock, wake) = &self.release;
            let mut released = lock.lock().expect("release lock");
            while !*released {
                released = wake.wait(released).expect("release wait");
            }
        }
        self.inner.install_inbound(virt, leg_c_port).map(|guard| self.wrap(guard))
    }
}

struct RegistrationRetiredDriver {
    inner: SimDriver,
    effects: Arc<parking_lot::Mutex<Vec<RegistrationRetiredEffect>>>,
    releases: parking_lot::Mutex<Vec<AllocationId>>,
    running: parking_lot::Mutex<Vec<AllocationId>>,
}

#[async_trait::async_trait]
impl Driver for RegistrationRetiredDriver {
    fn r#type(&self) -> DriverType {
        self.inner.r#type()
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        self.effects.lock().push(RegistrationRetiredEffect::DriverStart);
        self.inner.start(spec).await
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        self.effects.lock().push(RegistrationRetiredEffect::DriverStop);
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

    async fn release_for_exit_emission(&self, handle: &AllocationHandle) {
        self.releases.lock().push(handle.alloc.clone());
        self.inner.release_for_exit_emission(handle).await;
    }

    fn on_alloc_running(&self, spec: &AllocationSpec) {
        self.running.lock().push(spec.alloc.clone());
        self.inner.on_alloc_running(spec);
    }
}

struct RegistrationRetiredNetwork {
    alloc: AllocationId,
    allocator: Arc<NetSlotAllocator>,
    effects: Arc<parking_lot::Mutex<Vec<RegistrationRetiredEffect>>>,
    guard_drops: Arc<AtomicUsize>,
    fail_teardown: bool,
    teardown_slot_held: AtomicBool,
    teardown_guard_drops: AtomicUsize,
}

impl WorkloadNetworkProvisioner for RegistrationRetiredNetwork {
    fn provision(
        &self,
        _workload: &WorkloadNetnsPlan,
        _vm_tap: &VmTapPlan,
    ) -> Result<(), VethProvisionError> {
        self.effects.lock().push(RegistrationRetiredEffect::NetworkProvision);
        Ok(())
    }

    fn teardown(&self, _workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        self.teardown_slot_held
            .store(self.allocator.snapshot().contains_key(&self.alloc), Ordering::SeqCst);
        self.teardown_guard_drops.store(self.guard_drops.load(Ordering::SeqCst), Ordering::SeqCst);
        self.effects.lock().push(RegistrationRetiredEffect::NetworkTeardown);
        if self.fail_teardown {
            return Err(VethProvisionError::NetlinkConnect {
                source: overdrive_netlink::NetlinkError::Connect {
                    source: std::io::Error::from_raw_os_error(libc::EBUSY),
                },
            });
        }
        Ok(())
    }
}

struct RegistrationRetiredOutcome {
    rows: Vec<AllocStatusRow>,
    effects: Vec<RegistrationRetiredEffect>,
    releases: Vec<AllocationId>,
    running: Vec<AllocationId>,
    guard_drops: usize,
    teardown_slot_held: bool,
    teardown_guard_drops: usize,
    slot_still_held: bool,
}

async fn drive_registration_retired_through_action_shim(
    fail_teardown: bool,
) -> RegistrationRetiredOutcome {
    let tmp = TempDir::new().expect("tempdir");
    let store: Arc<dyn overdrive_core::traits::intent_store::IntentStore> =
        Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open store"));
    let obs = build_obs();
    let effects = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let intercept = Arc::new(RetirementBarrierIntercept::new(Arc::clone(&effects)));
    let worker = build_worker(Arc::clone(&intercept) as Arc<dyn MtlsIntercept>);
    worker.start_shared_owner().await.expect("publish the ordinary shared listener owner");

    let driver = Arc::new(RegistrationRetiredDriver {
        inner: SimDriver::new(DriverType::Vm),
        effects: Arc::clone(&effects),
        releases: parking_lot::Mutex::new(Vec::new()),
        running: parking_lot::Mutex::new(Vec::new()),
    });
    let drivers: Arc<overdrive_core::traits::driver::DriverRegistry> = {
        let mut registry = overdrive_core::traits::driver::DriverRegistry::new();
        registry.insert(Arc::clone(&driver) as Arc<dyn Driver>);
        Arc::new(registry)
    };
    let alloc_drivers = Arc::new(overdrive_control_plane::action_shim::AllocDriverIndex::default());
    let alloc = AllocationId::new(if fail_teardown {
        "registration-retired-cleanup-failure"
    } else {
        "registration-retired-cleanup-success"
    })
    .expect("valid allocation id");
    let allocator = Arc::new(NetSlotAllocator::new());
    let network = Arc::new(RegistrationRetiredNetwork {
        alloc: alloc.clone(),
        allocator: Arc::clone(&allocator),
        effects: Arc::clone(&effects),
        guard_drops: Arc::clone(&intercept.drops),
        fail_teardown,
        teardown_slot_held: AtomicBool::new(false),
        teardown_guard_drops: AtomicUsize::new(0),
    });
    let workload = WorkloadId::new("svc-registration-retired").expect("valid workload id");
    let node = NodeId::new("node-001").expect("valid node id");
    let mut spec = build_spec(&alloc);
    spec.service_ports = vec![NonZeroU16::new(8443).expect("non-zero port")];
    let action = Action::StartAllocation {
        alloc_id: alloc.clone(),
        workload_id: workload,
        node_id: node,
        spec,
        kind: WorkloadKind::Service,
    };
    let mut subscription = obs.subscribe_all_events().await.expect("subscribe before dispatch");

    let dispatch = tokio::spawn({
        let drivers = Arc::clone(&drivers);
        let alloc_drivers = Arc::clone(&alloc_drivers);
        let obs = Arc::clone(&obs);
        let store = Arc::clone(&store);
        let worker = Arc::clone(&worker);
        let allocator = Arc::clone(&allocator);
        let network = Arc::clone(&network);
        async move {
            let dataplane = overdrive_sim::adapters::dataplane::SimDataplane::new();
            let ca = overdrive_sim::adapters::ca::SimCa::new(Arc::new(
                overdrive_sim::adapters::entropy::SimEntropy::new(0),
            ));
            let clock = SimClock::new();
            let identity = overdrive_control_plane::identity_mgr::IdentityMgr::new(None);
            let (lifecycle_tx, _lifecycle_rx) = broadcast::channel(64);
            let broker =
                parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
            dispatch_with_network_provisioner(
                vec![action],
                drivers.as_ref(),
                alloc_drivers.as_ref(),
                obs.as_ref(),
                &dataplane,
                &ca,
                &clock,
                &identity,
                &lifecycle_tx,
                &tick_now(),
                &NodeId::new("writer-1").expect("node id"),
                build_vip_allocator(store),
                &broker,
                None,
                Some(&worker as &dyn MtlsInterceptLifecycle),
                allocator.as_ref(),
                network.as_ref(),
                &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
            )
            .await
        }
    });
    intercept.wait_entered().await;
    let retirement = tokio::spawn({
        let worker = Arc::clone(&worker);
        let alloc = alloc.clone();
        async move { worker.stop_alloc(&alloc).await }
    });
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert!(!retirement.is_finished(), "the retirement owner waits for Pending handoff");
    intercept.release();
    dispatch
        .await
        .expect("dispatch task joins")
        .expect("the production fail-closed arm records its terminal outcome");
    retirement
        .await
        .expect("retirement task joins")
        .expect("the action shim and external stop share one mTLS drain");
    let rows = drain_alloc_rows(&mut subscription, &alloc).await;
    let outcome = RegistrationRetiredOutcome {
        rows,
        effects: effects.lock().clone(),
        releases: driver.releases.lock().clone(),
        running: driver.running.lock().clone(),
        guard_drops: intercept.drops.load(Ordering::SeqCst),
        teardown_slot_held: network.teardown_slot_held.load(Ordering::SeqCst),
        teardown_guard_drops: network.teardown_guard_drops.load(Ordering::SeqCst),
        slot_still_held: allocator.snapshot().contains_key(&alloc),
    };
    allocator.release(&alloc);
    worker.shutdown_owner().await.expect("shared listener owner joins");
    outcome
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn registration_retired_from_real_start_alloc_keeps_exec_closed_and_releases_the_address_last()
 {
    for fail_teardown in [false, true] {
        let outcome = drive_registration_retired_through_action_shim(fail_teardown).await;
        assert_eq!(outcome.rows.len(), 2, "Running is superseded by one Failed receipt");
        assert_eq!(outcome.rows[0].state, AllocState::Running);
        assert_eq!(outcome.rows[1].state, AllocState::Failed);
        assert!(outcome.releases.is_empty(), "RegistrationRetired never releases EXEC");
        assert!(outcome.running.is_empty(), "the driver never observes accepted Running");
        assert_eq!(outcome.guard_drops, 2, "outbound and one inbound element drain exactly once");
        assert!(outcome.teardown_slot_held, "the address lease remains held during teardown");
        assert_eq!(outcome.teardown_guard_drops, 2, "mTLS drain precedes guest teardown");
        assert_eq!(
            outcome.effects,
            vec![
                RegistrationRetiredEffect::NetworkProvision,
                RegistrationRetiredEffect::DriverStart,
                RegistrationRetiredEffect::DriverStop,
                RegistrationRetiredEffect::MtlsElementDrop,
                RegistrationRetiredEffect::MtlsElementDrop,
                RegistrationRetiredEffect::NetworkTeardown,
            ]
        );
        if fail_teardown {
            assert!(outcome.slot_still_held, "failed teardown preserves the release-last lease");
            assert!(matches!(
                outcome.rows[1].reason,
                Some(TransitionReason::DriverInternalError { .. })
            ));
            let detail = outcome.rows[1].detail.as_deref().expect("aggregate cleanup detail");
            assert!(detail.contains("primary rejection"));
            assert!(detail.contains("retired before activation"));
            assert!(detail.contains("structural network teardown"));
        } else {
            assert!(!outcome.slot_still_held, "successful teardown releases the address last");
            assert!(matches!(
                outcome.rows[1].reason,
                Some(TransitionReason::MtlsInterceptInstallFailed { ref stage, .. })
                    if stage == "registration_retired"
            ));
        }
    }
}
