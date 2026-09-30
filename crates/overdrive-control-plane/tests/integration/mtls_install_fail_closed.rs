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
//! The S-ND295-52 bodies drive the post-#295 action owner through
//! `dispatch_with_guest_network_provisioner_for_test` over the seam fixture of
//! `test-scenarios.md` § *Seam fixture* (one owner instance, one EXEC wiring
//! opened by its paired supervisor, one pool, one worker over sim ports). They
//! observe one ordered journal written by every participant at its own driven
//! port — the owner, the VM-shaped driver, the intercept — plus the pinned
//! structured events (`mtls.intercept.install.success`,
//! `guest_network.lease_retired`, `guest_network.lease_released`). Where an
//! oracle was active before #295, its retained assertions stay in an active
//! body and the #295 additions (lease events, the EXEC-gate wait) are a sibling
//! body pending the step that lands them, so no retained regression goes
//! unguarded in between. These seam bodies use sim ports only and need no
//! privilege.
//!
//! Every body that drives an arm over a structural network adapter calls the
//! test-gated `dispatch_with_network_provisioner` through the one helper
//! [`dispatch_over_network_provisioner`], so the R16 change of its lifecycle
//! parameter edits one line. The S-ND295-54 restart-abort detail bodies enter a
//! successor stop failure there through a test-local `MtlsInterceptLifecycle`.
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
//! scenario would silently exercise the wrong handler. The historical S-MIF
//! scenarios therefore SKIP (not fail) off root and print an explicit EXECUTED
//! marker past the gate, so a skipped run is never mistaken for a pass. The
//! #295 bodies over the seam fixture reach no netns and carry no root gate.
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
use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex as StdMutex};
use std::time::{Duration, Instant};

use futures::StreamExt;
use tokio::sync::broadcast;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use overdrive_control_plane::action_shim::{
    MtlsInterceptLifecycle, ShimError, WorkloadNetworkProvisioner, dispatch,
    dispatch_with_guest_network_provisioner_for_test, dispatch_with_network_provisioner,
};
use overdrive_control_plane::guest_network::{
    GuestAddressPool, GuestLinkKind, GuestNetworkError, GuestNetworkFact, GuestNetworkOperation,
    GuestNetworkPlan, GuestNetworkProvisioner, SharedGuestNetworkAudit,
    SharedGuestNetworkAuditError, SharedGuestNetworkOwner, TapActivation, TapQuiescence,
};
use overdrive_control_plane::veth_provisioner::{
    NetSlot, NetSlotAllocator, VethProvisionError, VmTapPlan, WorkloadNetnsPlan,
    derive_workload_netns_plan, responder_addr_for_slot, teardown_workload_netns,
};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::guest_network::{
    GuestNetworkExecGate, GuestNetworkExecSupervisor, GuestNetworkExecWiring,
    SharedGuestNetworkComponent,
};
use overdrive_core::id::{AllocationId, CertSerial, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverStartClass,
    DriverStartFailure, DriverType, Resources,
};
use overdrive_core::traits::mtls_enforcement::{
    EnforcedConnectionId, MtlsEnforcement, MtlsEnforcementError, MtlsLimits,
};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LagAwareSubscription, LogicalTimestamp, ObservationRow,
    ObservationStore, ObservationStoreError, SubscriptionEvent,
};
use overdrive_core::transition_reason::TransitionReason;

use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_sim::adapters::SimIdentityRead;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
use overdrive_sim::adapters::mtls_intercept::{SimInterceptFault, SimMtlsIntercept};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::mtls_intercept::{
    InterceptElementKey, InterceptElementOperation, InterceptError, InterceptPostcondition,
    InterceptSet, NetlinkError,
};
use overdrive_worker::mtls_intercept_port::{InterceptGuard, MtlsIntercept};
use overdrive_worker::mtls_intercept_worker::{
    HandleTeardownFailure, MtlsInterceptInstallError, MtlsInterceptStopError, MtlsInterceptWorker,
};
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
    stops: parking_lot::Mutex<Vec<AllocationId>>,
    releases: parking_lot::Mutex<Vec<AllocationId>>,
    on_alloc_running_calls: parking_lot::Mutex<Vec<AllocationId>>,
}

impl RecordingDriver {
    fn new() -> Self {
        Self {
            inner: SimDriver::new(DriverType::Vm),
            starts: parking_lot::Mutex::new(Vec::new()),
            stops: parking_lot::Mutex::new(Vec::new()),
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
        self.stops.lock().push(handle.alloc.clone());
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

// ---------------------------------------------------------------------------
// S-ND295-52 test support: one ordered journal written by every participant
// at its own driven port, and the seam fixture of `test-scenarios.md`
// § *Seam fixture*.
// ---------------------------------------------------------------------------

/// One observed step of the action owner's start / cleanup sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// The owner's `provision`.
    Provision,
    /// `Driver::start`.
    DriverStart,
    /// `start_alloc` installed the allocation's intercept elements.
    InterceptInstalled,
    /// The synchronous `mtls.intercept.install.success` event.
    InstallSuccessEvent,
    /// The owner's `activate`.
    Activate,
    /// `Driver::stop`.
    DriverStop,
    /// `guest_network.lease_retired`.
    LeaseRetired,
    /// `stop_alloc` released the allocation's intercept elements.
    ElementRelease,
    /// The owner's `teardown`.
    Teardown,
    /// `guest_network.lease_released`.
    LeaseReleased,
    /// `Driver::release_for_exit_emission` (the EXEC release).
    ReleaseForExitEmission,
    /// `Driver::on_alloc_running`.
    OnAllocRunning,
}

#[derive(Clone, Debug)]
struct JournalEntry {
    step: Step,
    /// The allocation the step names, when the participant knows it.
    alloc: Option<String>,
}

/// Append-only, ordered across every participant.
type Journal = Arc<parking_lot::Mutex<Vec<JournalEntry>>>;

fn record(journal: &Journal, step: Step, alloc: Option<&AllocationId>) {
    journal.lock().push(JournalEntry { step, alloc: alloc.map(ToString::to_string) });
}

/// The journal's steps for `alloc` (steps that name no allocation included),
/// with repeated consecutive element steps collapsed to one.
fn steps_for(journal: &Journal, alloc: &AllocationId) -> Vec<Step> {
    let entries = journal.lock().clone();
    for entry in &entries {
        eprintln!("[journal] {:?} {:?}", entry.step, entry.alloc);
    }
    let mut steps: Vec<Step> = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry.alloc.as_deref().is_some_and(|named| named != alloc.as_str()) {
            continue;
        }
        let collapses = matches!(entry.step, Step::InterceptInstalled | Step::ElementRelease)
            && steps.last() == Some(&entry.step);
        if !collapses {
            steps.push(entry.step);
        }
    }
    steps
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

/// Records the install-success event and the pinned lease events
/// (`guest_network.lease_retired` / `lease_released`, FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the lease events)).
#[derive(Clone)]
struct JournalLayer(Journal);

impl<S> Layer<S> for JournalLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut fields = EventFields::default();
        event.record(&mut fields);
        let name = fields
            .0
            .get("event")
            .or_else(|| fields.0.get("name"))
            .cloned()
            .unwrap_or_else(|| event.metadata().name().to_owned());
        let step = match name.as_str() {
            "mtls.intercept.install.success" => Step::InstallSuccessEvent,
            "guest_network.lease_retired" => Step::LeaseRetired,
            "guest_network.lease_released" => Step::LeaseReleased,
            _ => return,
        };
        self.0.lock().push(JournalEntry { step, alloc: fields.0.get("alloc").cloned() });
    }
}

/// VM-shaped sim driver recording every lifecycle call in the journal.
struct JournalDriver {
    inner: SimDriver,
    journal: Journal,
}

#[async_trait::async_trait]
impl Driver for JournalDriver {
    fn r#type(&self) -> DriverType {
        self.inner.r#type()
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        record(&self.journal, Step::DriverStart, Some(&spec.alloc));
        self.inner.start(spec).await
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        record(&self.journal, Step::DriverStop, Some(&handle.alloc));
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
        record(&self.journal, Step::ReleaseForExitEmission, Some(&handle.alloc));
        self.inner.release_for_exit_emission(handle).await;
    }

    fn on_alloc_running(&self, spec: &AllocationSpec) {
        record(&self.journal, Step::OnAllocRunning, Some(&spec.alloc));
        self.inner.on_alloc_running(spec);
    }
}

/// Element guard that journals its release.
struct JournalGuard {
    _inner: Box<dyn InterceptGuard>,
    journal: Journal,
}

impl InterceptGuard for JournalGuard {}

impl Drop for JournalGuard {
    fn drop(&mut self) {
        record(&self.journal, Step::ElementRelease, None);
    }
}

/// The listener type `MtlsIntercept::bind_transparent` returns. The DELIVER
/// step that carries B-7 (05-01 at the latest) changes it to
/// `Arc<dyn InterceptListener>` (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the pinned `bind_transparent` signature)); the delegation below is
/// unchanged by that step.
type BoundListener = std::net::TcpListener;

/// Test-local intercept over `SimMtlsIntercept` that journals element
/// installation and release; `bind_transparent` delegates to the sim.
struct JournalIntercept {
    inner: SimMtlsIntercept,
    journal: Journal,
}

impl JournalIntercept {
    fn guard(&self, inner: Box<dyn InterceptGuard>) -> Box<dyn InterceptGuard> {
        Box::new(JournalGuard { _inner: inner, journal: Arc::clone(&self.journal) })
    }
}

impl MtlsIntercept for JournalIntercept {
    fn bind_transparent(
        &self,
        address: SocketAddrV4,
    ) -> overdrive_worker::mtls_intercept::Result<BoundListener> {
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
        source_addr: Ipv4Addr,
        leg_f_port: u16,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        let guard = self.inner.install_outbound(source_addr, leg_f_port)?;
        record(&self.journal, Step::InterceptInstalled, None);
        Ok(self.guard(guard))
    }

    fn install_inbound(
        &self,
        virt: SocketAddrV4,
        leg_c_port: u16,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        let guard = self.inner.install_inbound(virt, leg_c_port)?;
        record(&self.journal, Step::InterceptInstalled, None);
        Ok(self.guard(guard))
    }

    fn observe_shared_state(
        &self,
    ) -> overdrive_worker::mtls_intercept::Result<
        Option<overdrive_worker::mtls_intercept_port::InterceptState>,
    > {
        self.inner.observe_shared_state()
    }

    fn converge_allocation_elements(
        &self,
        expected: &overdrive_worker::mtls_intercept_port::InterceptMembers,
    ) -> overdrive_worker::mtls_intercept::Result<
        Option<overdrive_worker::mtls_intercept_port::InterceptState>,
    > {
        self.inner.converge_allocation_elements(expected)
    }

    fn remove_allocation_elements(
        &self,
        source_addr: Ipv4Addr,
        destinations: &[SocketAddrV4],
    ) -> overdrive_worker::mtls_intercept::Result<
        overdrive_worker::mtls_intercept_port::InterceptState,
    > {
        record(&self.journal, Step::ElementRelease, None);
        self.inner.remove_allocation_elements(source_addr, destinations)
    }
}

/// One `activate` refusal as the owner port returned it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ActivationRefusal {
    /// The allocation the plan names.
    alloc: AllocationId,
    /// The TAP the plan's assignment names.
    tap: String,
    /// `(operation, expected, observed)` when the refusal is a source-less
    /// `GuestNetworkError::PostconditionMismatch`.
    mismatch: Option<(GuestNetworkOperation, GuestNetworkFact, Option<GuestNetworkFact>)>,
}

/// The one owner instance: a journaling decorator over the pinned
/// `SimSharedGuestNetworkOwner`. With `refuse_activation`, `activate` returns
/// a typed post-set-up mismatch instead of delegating. When an activation hold
/// is armed, the next `activate` is journalled on entry and then waits for one
/// permit before it reaches the owner (an activation in flight). Every
/// activation refusal is recorded as the port returned it.
struct JournalOwner {
    inner: Arc<SimSharedGuestNetworkOwner>,
    journal: Journal,
    refuse_activation: bool,
    activation_hold: parking_lot::Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    activations_held: AtomicUsize,
    refusals: parking_lot::Mutex<Vec<ActivationRefusal>>,
}

impl JournalOwner {
    const fn new(
        inner: Arc<SimSharedGuestNetworkOwner>,
        journal: Journal,
        refuse_activation: bool,
    ) -> Self {
        Self {
            inner,
            journal,
            refuse_activation,
            activation_hold: parking_lot::Mutex::new(None),
            activations_held: AtomicUsize::new(0),
            refusals: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Hold the next `activate` before it reaches the owner until one permit
    /// is added to the returned semaphore.
    fn hold_next_activation(&self) -> Arc<tokio::sync::Semaphore> {
        let hold = Arc::new(tokio::sync::Semaphore::new(0));
        *self.activation_hold.lock() = Some(Arc::clone(&hold));
        hold
    }

    /// `activate` calls currently waiting on a hold.
    fn activations_held(&self) -> usize {
        self.activations_held.load(Ordering::SeqCst)
    }

    /// Every activation refusal, in call order.
    fn refusals(&self) -> Vec<ActivationRefusal> {
        self.refusals.lock().clone()
    }

    fn record_refusal(&self, plan: &GuestNetworkPlan, error: &GuestNetworkError) {
        let mismatch = match error {
            GuestNetworkError::PostconditionMismatch { operation, expected, observed } => {
                Some((*operation, expected.clone(), observed.clone()))
            }
            _ => None,
        };
        self.refusals.lock().push(ActivationRefusal {
            alloc: plan.alloc().clone(),
            tap: plan.assignment().tap.clone(),
            mismatch,
        });
    }
}

#[async_trait::async_trait]
impl GuestNetworkProvisioner for JournalOwner {
    async fn provision(
        &self,
        plan: &GuestNetworkPlan,
    ) -> overdrive_control_plane::guest_network::Result<()> {
        record(&self.journal, Step::Provision, Some(plan.alloc()));
        self.inner.provision(plan).await
    }

    async fn activate(
        &self,
        plan: &GuestNetworkPlan,
    ) -> overdrive_control_plane::guest_network::Result<TapActivation> {
        record(&self.journal, Step::Activate, Some(plan.alloc()));
        let hold = self.activation_hold.lock().take();
        if let Some(hold) = hold {
            self.activations_held.fetch_add(1, Ordering::SeqCst);
            hold.acquire().await.expect("the activation hold is never closed").forget();
            self.activations_held.fetch_sub(1, Ordering::SeqCst);
        }
        let result = if self.refuse_activation {
            Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::TapSetUp,
                expected: GuestNetworkFact::LinkUp { ifindex: 295, up: true },
                observed: Some(GuestNetworkFact::LinkUp { ifindex: 295, up: false }),
            })
        } else {
            self.inner.activate(plan).await
        };
        if let Err(error) = &result {
            self.record_refusal(plan, error);
        }
        result
    }

    async fn teardown(
        &self,
        plan: &GuestNetworkPlan,
    ) -> overdrive_control_plane::guest_network::Result<()> {
        record(&self.journal, Step::Teardown, Some(plan.alloc()));
        self.inner.teardown(plan).await
    }
}

#[async_trait::async_trait]
impl SharedGuestNetworkOwner for JournalOwner {
    async fn probe_startup(&self) -> overdrive_control_plane::guest_network::Result<()> {
        self.inner.probe_startup().await
    }

    async fn sweep_stale(&self) -> overdrive_control_plane::guest_network::Result<()> {
        self.inner.sweep_stale().await
    }

    async fn converge_shared(&self) -> overdrive_control_plane::guest_network::Result<()> {
        self.inner.converge_shared().await
    }

    async fn audit_shared(&self) -> Result<SharedGuestNetworkAudit, SharedGuestNetworkAuditError> {
        self.inner.audit_shared().await
    }

    async fn quiesce_managed_taps(
        &self,
    ) -> overdrive_control_plane::guest_network::Result<TapQuiescence> {
        self.inner.quiesce_managed_taps().await
    }

    async fn restore_quiesced_taps(&self) -> overdrive_control_plane::guest_network::Result<()> {
        self.inner.restore_quiesced_taps().await
    }
}

/// The seam fixture (`test-scenarios.md` § *Seam fixture*): one owner
/// instance, one EXEC wiring over the fixture clock, one pool from
/// `GuestAddressPool::new`, and one worker over sim ports.
///
/// Until DELIVER 05-01 cuts the pinned `AppState` constructors (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the `AppState` constructors)),
/// `AppState::new` takes today's inputs, and the owner reaches dispatch only
/// as the seam's `provisioner`; the gate and pool are held here. 05-01 changes
/// only the constructor call in [`SeamFixture::new`]: it passes `worker`,
/// `owner`, `wiring.gate()`, and `guest_pool` to `AppState::new`, so
/// `AppState`'s owner and the seam's provisioner are the one instance.
struct SeamFixture {
    _tmp: TempDir,
    state: Arc<overdrive_control_plane::AppState>,
    obs: Arc<SimObservationStore>,
    worker: Arc<MtlsInterceptWorker>,
    owner: Arc<JournalOwner>,
    supervisor: Arc<GuestNetworkExecSupervisor>,
    _exec_gate: Arc<GuestNetworkExecGate>,
    _guest_pool: Arc<GuestAddressPool>,
}

impl SeamFixture {
    async fn new(
        driver: Arc<dyn Driver>,
        intercept: Arc<dyn MtlsIntercept>,
        owner: Arc<JournalOwner>,
        identity: Arc<overdrive_control_plane::identity_mgr::IdentityMgr>,
    ) -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("open store"));
        let obs = build_obs();
        let clock: Arc<SimClock> = Arc::new(SimClock::new());
        let wiring = GuestNetworkExecWiring::new(Arc::clone(&clock) as _);
        let worker = build_worker(intercept);
        let guest_pool = Arc::new(GuestAddressPool::new(
            ipnet::Ipv4Net::new(Ipv4Addr::new(100, 95, 0, 0), 16).expect("node guest prefix"),
            "ovd-gbr0".to_owned(),
            Ipv4Addr::new(100, 95, 0, 1),
            Ipv4Addr::new(100, 95, 0, 1),
        ));
        let mut runtime =
            overdrive_control_plane::reconciler_runtime::ReconcilerRuntime::new_with_redb_view_store_for_test(
                tmp.path(),
            )
            .expect("runtime");
        runtime
            .register(overdrive_control_plane::noop_heartbeat())
            .await
            .expect("register heartbeat");
        let mut state = overdrive_control_plane::AppState::new(
            Arc::clone(&store),
            store_path,
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::new(runtime),
            driver,
            clock,
            Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
            Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
                overdrive_sim::adapters::entropy::SimEntropy::new(295),
            ))),
            identity,
            NodeId::new("node-001").expect("node id"),
            build_vip_allocator(
                Arc::clone(&store) as Arc<dyn overdrive_core::traits::intent_store::IntentStore>
            ),
            overdrive_control_plane::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
        );
        state.mtls_worker = Some(Arc::clone(&worker));
        Self {
            _tmp: tmp,
            state: Arc::new(state),
            obs,
            worker,
            owner,
            supervisor: wiring.supervisor(),
            _exec_gate: wiring.gate(),
            _guest_pool: guest_pool,
        }
    }

    /// Open the fixture's EXEC gate through its paired supervisor, as a
    /// dispatch that reaches `claim_release` requires.
    fn open_exec(&self) {
        assert!(self.supervisor.open_after_boot(), "the fixture's gate leaves BootClosed once");
    }

    async fn dispatch(&self, action: Action) -> Result<(), ShimError> {
        let tick = tick_now();
        Box::pin(dispatch_with_guest_network_provisioner_for_test(
            vec![action],
            self.state.as_ref(),
            &tick,
            self.owner.as_ref(),
        ))
        .await
    }
}

/// A seam fixture over the journaling participants, with the journal layer
/// installed as the thread's default subscriber for the returned guard's
/// lifetime.
async fn journaled_fixture(
    inner_owner: Arc<SimSharedGuestNetworkOwner>,
    refuse_activation: bool,
) -> (SeamFixture, Journal, tracing::subscriber::DefaultGuard) {
    let journal: Journal = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let guard = tracing::subscriber::set_default(
        tracing_subscriber::registry().with(JournalLayer(Arc::clone(&journal))),
    );
    let fixture = SeamFixture::new(
        Arc::new(JournalDriver {
            inner: SimDriver::new(DriverType::Vm),
            journal: Arc::clone(&journal),
        }),
        Arc::new(JournalIntercept {
            inner: SimMtlsIntercept::new(),
            journal: Arc::clone(&journal),
        }),
        Arc::new(JournalOwner::new(inner_owner, Arc::clone(&journal), refuse_activation)),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
    )
    .await;
    (fixture, journal, guard)
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
// Rejected initial Running writes. Network provisioning precedes
// Driver::start, so the provisioned network owner must unwind even though the
// observation store cannot record the initial Running row.
//
// The two RETAINED bodies keep the structural-unwind oracle they carried while
// active (at `3cc00933`): over the veth provisioner, the rejection surfaces,
// the driver started once, the one provisioned owner is torn down, and its slot
// is released. The lease-event contract that the shared guest-network owner
// adds (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the retirement points and the lease events)) is
// asserted by the two sibling bodies over the seam fixture, pending 06-03.
// ---------------------------------------------------------------------------

/// A structural network adapter that counts its calls and never fails.
struct CountingNetwork {
    provisions: AtomicUsize,
    teardowns: AtomicUsize,
}

impl CountingNetwork {
    const fn new() -> Self {
        Self { provisions: AtomicUsize::new(0), teardowns: AtomicUsize::new(0) }
    }
}

impl WorkloadNetworkProvisioner for CountingNetwork {
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

/// The one call site of `dispatch_with_network_provisioner` in this file. Every
/// body that drives an arm over a structural network adapter goes through
/// here, with every orthogonal port a sim double. The R16 cut (DELIVER 05-01)
/// changes the lifecycle parameter from `Option<&dyn MtlsInterceptLifecycle>`
/// to `&dyn MtlsInterceptLifecycle` (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (there is no activation
/// path without intercept-live)); that cut edits exactly the one marked line.
#[allow(
    clippy::too_many_arguments,
    reason = "mirrors the test-gated dispatch entry's required driving-port inputs"
)]
async fn dispatch_over_network_provisioner(
    actions: Vec<Action>,
    drivers: &overdrive_core::traits::driver::DriverRegistry,
    alloc_drivers: &overdrive_control_plane::action_shim::AllocDriverIndex,
    obs: &dyn ObservationStore,
    identity: &overdrive_control_plane::identity_mgr::IdentityMgr,
    store: Arc<dyn overdrive_core::traits::intent_store::IntentStore>,
    mtls_lifecycle: &dyn MtlsInterceptLifecycle,
    net_slots: &NetSlotAllocator,
    network: &dyn WorkloadNetworkProvisioner,
) -> Result<(), ShimError> {
    let dataplane = overdrive_sim::adapters::dataplane::SimDataplane::new();
    let ca = overdrive_sim::adapters::ca::SimCa::new(Arc::new(
        overdrive_sim::adapters::entropy::SimEntropy::new(0),
    ));
    let clock = SimClock::new();
    let (lifecycle_tx, _lifecycle_rx) = broadcast::channel(64);
    let broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    dispatch_with_network_provisioner(
        actions,
        drivers,
        alloc_drivers,
        obs,
        &dataplane,
        &ca,
        &clock,
        identity,
        &lifecycle_tx,
        &tick_now(),
        &NodeId::new("writer-1").expect("node id"),
        build_vip_allocator(store),
        &broker,
        None,
        // DELIVER 05-01 (R16): this line becomes `mtls_lifecycle`.
        Some(mtls_lifecycle),
        net_slots,
        network,
        &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
    )
    .await
}

struct RunningWriteRejectOutcome {
    result: Result<(), ShimError>,
    starts: Vec<AllocationId>,
    provisions: usize,
    teardowns: usize,
    slot_still_held: bool,
}

/// Drive the production start/restart arm through a rejected initial Running
/// write after provision and driver start, over a deterministic structural
/// network adapter rather than host network privileges.
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
    let network = CountingNetwork::new();
    let result = dispatch_over_network_provisioner(
        vec![action],
        drivers.as_ref(),
        &alloc_drivers,
        obs.as_ref(),
        &identity,
        store,
        &worker,
        &net_slots,
        &network,
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
    assert_eq!(outcome.provisions, 1, "{scenario}: exactly one network owner is provisioned");
    assert_eq!(outcome.teardowns, 1, "{scenario}: rejected write must tear that owner down");
    assert!(
        !outcome.slot_still_held,
        "{scenario}: structural teardown must release the allocation's slot"
    );
}

/// A rejected fresh-start Running write unwinds its network owner.
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

struct LeaseRunningWriteRejectOutcome {
    result: Result<(), ShimError>,
    alloc: AllocationId,
    steps: Vec<Step>,
}

/// Drive the production start/restart arm through a rejected initial Running
/// write after provision and driver start, over the seam fixture and its one
/// shared guest-network owner.
async fn drive_running_write_rejection_over_the_seam(arm: Arm) -> LeaseRunningWriteRejectOutcome {
    let (fixture, journal, _trace_guard) =
        journaled_fixture(Arc::new(SimSharedGuestNetworkOwner::default()), false).await;
    fixture.open_exec();
    let alloc = AllocationId::new(match arm {
        Arm::Start => "running-write-reject-start",
        Arm::Restart => "running-write-reject-restart",
    })
    .expect("valid allocation id");
    let predecessor = AllocationId::new("running-write-reject-restart-predecessor")
        .expect("valid predecessor allocation id");
    let successor = AllocationId::new("running-write-reject-restart-successor")
        .expect("valid successor allocation id");
    let effect_alloc = if matches!(arm, Arm::Start) { alloc.clone() } else { successor.clone() };
    let workload = WorkloadId::new("svc-running-write-reject").expect("valid workload id");
    let node = NodeId::new("node-001").expect("valid node id");
    fixture.state.identity.hold(effect_alloc.clone(), held_svid(&workload, &effect_alloc));
    let action = match arm {
        Arm::Start => Action::StartAllocation {
            alloc_id: alloc.clone(),
            workload_id: workload,
            node_id: node,
            spec: build_spec(&alloc),
            kind: WorkloadKind::Service,
        },
        Arm::Restart => {
            seed_restart_predecessor(fixture.obs.as_ref(), &predecessor, &workload, &node).await;
            Action::RestartAllocation {
                alloc_id: predecessor,
                spec: build_spec(&successor),
                kind: WorkloadKind::Service,
            }
        }
    };
    fixture.obs.inject_write_failure(ObservationStoreError::Unreachable {
        peer: "rejected-running-write".to_owned(),
    });
    let result = fixture.dispatch(action).await;
    let steps = steps_for(&journal, &effect_alloc);
    LeaseRunningWriteRejectOutcome { result, alloc: effect_alloc, steps }
}

fn position(steps: &[Step], step: Step) -> Option<usize> {
    steps.iter().position(|candidate| *candidate == step)
}

fn assert_running_write_rejection_retires_then_releases(
    scenario: &str,
    outcome: &LeaseRunningWriteRejectOutcome,
) {
    assert!(
        matches!(&outcome.result, Err(ShimError::Observation(_))),
        "{scenario}: the injected initial Running write rejection must surface; got {:?}",
        outcome.result
    );
    let steps = &outcome.steps;
    assert_eq!(
        steps.get(..2),
        Some([Step::Provision, Step::DriverStart].as_slice()),
        "{scenario}: provision precedes the one driver start of {}; journal {steps:?}",
        outcome.alloc
    );
    for once in [Step::DriverStop, Step::LeaseRetired, Step::Teardown, Step::LeaseReleased] {
        assert_eq!(
            steps.iter().filter(|step| **step == once).count(),
            1,
            "{scenario}: exactly one {once:?} for {}; journal {steps:?}",
            outcome.alloc
        );
    }
    let driver_stop = position(steps, Step::DriverStop);
    let retired = position(steps, Step::LeaseRetired);
    let teardown = position(steps, Step::Teardown);
    let released = position(steps, Step::LeaseReleased);
    assert!(
        driver_stop < teardown && retired < teardown && teardown < released,
        "{scenario}: the lease is Retiring before the attachment's teardown, which precedes its \
         release; journal {steps:?}"
    );
    assert_eq!(
        steps.last(),
        Some(&Step::LeaseReleased),
        "{scenario}: the lease is released last; journal {steps:?}"
    );
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-06 — Start publishes no partial attachment
/// A rejected fresh-start Running write over the shared guest-network owner
/// retires the lease before its attachment's teardown and releases it last.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 06-03 (S-ND295-06)"]
async fn start_running_write_rejection_retires_the_lease_before_teardown_and_releases_it_last() {
    let outcome = drive_running_write_rejection_over_the_seam(Arm::Start).await;
    assert_running_write_rejection_retires_then_releases("fresh start", &outcome);
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-06 — Start publishes no partial attachment
/// A rejected restarted Running write over the shared guest-network owner
/// follows the same retire-then-release unwind for the successor.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 06-03 (S-ND295-06)"]
async fn restart_running_write_rejection_retires_the_lease_before_teardown_and_releases_it_last() {
    let outcome = drive_running_write_rejection_over_the_seam(Arm::Restart).await;
    assert_running_write_rejection_retires_then_releases("restart", &outcome);
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

/// The accepted failure projection of an activation `Err` (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the activation failure projection)):
/// `driver.stop`, lease retired, `stop_alloc`, teardown, lease released last;
/// a dominating Failed row; zero EXEC release.
async fn assert_activation_failure_projection(
    scenario: &str,
    fixture: &SeamFixture,
    journal: &Journal,
    alloc: &AllocationId,
) {
    let steps = steps_for(journal, alloc);
    assert_eq!(
        steps,
        [
            Step::Provision,
            Step::DriverStart,
            Step::InterceptInstalled,
            Step::InstallSuccessEvent,
            Step::Activate,
            Step::DriverStop,
            Step::LeaseRetired,
            Step::ElementRelease,
            Step::Teardown,
            Step::LeaseReleased,
        ],
        "{scenario}: an activation failure stops the VMM, retires the lease, stops protection, \
         tears down, and releases the lease last, never releasing EXEC"
    );
    assert_eq!(fixture.worker.stop_alloc_calls_for_test(), 1, "{scenario}: one protection stop");
    let row = fixture
        .obs
        .alloc_status_row(alloc)
        .await
        .expect("activation-failure row read succeeds")
        .expect("activation-failure row is retained");
    assert_eq!(row.state, AllocState::Failed, "{scenario}: the Failed row dominates Running");
    assert!(
        matches!(
            row.reason,
            Some(TransitionReason::WorkloadNetnsProvisionFailed { ref stage, .. })
                if stage == "guest_network_activate"
        ),
        "{scenario}: the Failed row names the activation stage; got {:?}",
        row.reason
    );
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-52 — The action shim raises the TAP after the protection-live event and before the command
/// CONTRACT_SHAPE: bounded-change.
///
/// The RETAINED ordering: start_alloc, the protection-live event, activation,
/// the EXEC release, then the running hook. It holds with or without the EXEC
/// gate wait; the wait itself is the sibling body pending 06-04. The seam
/// fixture is all sim ports and needs no privilege.
#[tokio::test]
async fn tap_activation_occurs_after_intercept_success_and_before_exec_release() {
    let (fixture, journal, _trace_guard) =
        journaled_fixture(Arc::new(SimSharedGuestNetworkOwner::default()), false).await;
    fixture.worker.start_shared_owner().await.expect("the worker's shared owner is healthy");
    fixture.open_exec();
    let alloc = AllocationId::new("gti-activation-order").expect("valid alloc id");
    let action = Action::StartAllocation {
        alloc_id: alloc.clone(),
        workload_id: WorkloadId::new("svc-gti-activation-order").expect("valid workload id"),
        node_id: NodeId::new("node-001").expect("valid node id"),
        spec: build_spec(&alloc),
        kind: WorkloadKind::Service,
    };

    fixture.dispatch(action).await.expect("the activated start completes");

    assert_eq!(
        steps_for(&journal, &alloc),
        [
            Step::Provision,
            Step::DriverStart,
            Step::InterceptInstalled,
            Step::InstallSuccessEvent,
            Step::Activate,
            Step::ReleaseForExitEmission,
            Step::OnAllocRunning,
        ],
        "start_alloc, then the protection-live event, then activation, then the EXEC release, \
         then the running hook"
    );

    fixture.worker.stop_alloc(&alloc).await.expect("allocation protection teardown succeeds");
    fixture.worker.shutdown_owner().await.expect("the shared owner shuts down");
}

/// Whether the journal holds `step` for `alloc` (silent, for polling).
fn journal_has(journal: &Journal, alloc: &AllocationId, step: Step) -> bool {
    journal.lock().iter().any(|entry| {
        entry.step == step && entry.alloc.as_deref().is_none_or(|a| a == alloc.as_str())
    })
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-52 — The action shim raises the TAP after the protection-live event and before the command
/// CONTRACT_SHAPE: bounded-change.
///
/// The activation step claims the EXEC gate first (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the
/// action-shim order; waiting on the EXEC gate)): with the fixture's gate
/// Recovering, the start parks after the protection-live event and reaches no
/// `activate`, no EXEC release, and no running hook; the reopen lets it
/// activate exactly once, release the command, and keep its Running row.
#[tokio::test]
#[ignore = "pending DELIVER step 06-04 (S-ND295-52)"]
async fn tap_activation_waits_on_a_recovering_exec_gate_and_runs_once_after_reopen() {
    let (fixture, journal, _trace_guard) =
        journaled_fixture(Arc::new(SimSharedGuestNetworkOwner::default()), false).await;
    let fixture = Arc::new(fixture);
    fixture.worker.start_shared_owner().await.expect("the worker's shared owner is healthy");
    fixture.open_exec();
    assert!(
        fixture.supervisor.begin_recovery(SharedGuestNetworkComponent::Bridge),
        "the fixture's paired supervisor moves the gate from Open to Recovering"
    );
    let alloc = AllocationId::new("gti-activation-waits").expect("valid alloc id");
    let action = Action::StartAllocation {
        alloc_id: alloc.clone(),
        workload_id: WorkloadId::new("svc-gti-activation-waits").expect("valid workload id"),
        node_id: NodeId::new("node-001").expect("valid node id"),
        spec: build_spec(&alloc),
        kind: WorkloadKind::Service,
    };
    let task = {
        let fixture = Arc::clone(&fixture);
        tokio::spawn(async move { fixture.dispatch(action).await })
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        while !journal_has(&journal, &alloc, Step::InstallSuccessEvent) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the start reaches the protection-live event");
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
    assert!(!task.is_finished(), "the start waits on the recovering EXEC gate");
    assert_eq!(
        steps_for(&journal, &alloc),
        [Step::Provision, Step::DriverStart, Step::InterceptInstalled, Step::InstallSuccessEvent],
        "while Recovering there is no activation, no EXEC release, and no running hook"
    );

    assert!(
        fixture.supervisor.complete_attempt(None),
        "the recovery completes and reopens the gate"
    );
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("the start completes after the reopen")
        .expect("the dispatch task joins")
        .expect("the activated start completes");
    assert_eq!(
        steps_for(&journal, &alloc),
        [
            Step::Provision,
            Step::DriverStart,
            Step::InterceptInstalled,
            Step::InstallSuccessEvent,
            Step::Activate,
            Step::ReleaseForExitEmission,
            Step::OnAllocRunning,
        ],
        "after the reopen: exactly one activation, then the EXEC release, then the running hook"
    );
    let row = fixture
        .obs
        .alloc_status_row(&alloc)
        .await
        .expect("row read succeeds")
        .expect("the started allocation has a row");
    assert_eq!(row.state, AllocState::Running, "an observed recovery writes no Failed row");

    fixture.worker.stop_alloc(&alloc).await.expect("allocation protection teardown succeeds");
    fixture.worker.shutdown_owner().await.expect("the shared owner shuts down");
}

/// Driver double whose async EXEC release is deliberately held. Its drop
/// marker makes cancellation observable without starting a second task inside
/// the method.
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

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-27 / S-ND295-28 — One guest-command gate; recovery never leaks or wrongly releases a command
/// CONTRACT_SHAPE: bounded-change.
///
/// The RETAINED action-shim await/cancellation contract, kept as its own body
/// when S-ND295-52 retargeted its former home: after activation the start arm
/// awaits the async EXEC release in place; cancelling the dispatch while that
/// release is held drops the same release future, and neither the release nor
/// the running hook completes afterwards.
#[tokio::test]
async fn cancelling_dispatch_while_the_exec_release_is_held_drops_that_release() {
    let journal: Journal = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let _trace_guard = tracing::subscriber::set_default(
        tracing_subscriber::registry().with(JournalLayer(Arc::clone(&journal))),
    );
    let driver = Arc::new(HoldingReleaseDriver::new());
    let fixture = Arc::new(
        SeamFixture::new(
            Arc::clone(&driver) as Arc<dyn Driver>,
            Arc::new(JournalIntercept {
                inner: SimMtlsIntercept::new(),
                journal: Arc::clone(&journal),
            }),
            Arc::new(JournalOwner::new(
                Arc::new(SimSharedGuestNetworkOwner::default()),
                Arc::clone(&journal),
                false,
            )),
            Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        )
        .await,
    );
    fixture.worker.start_shared_owner().await.expect("the worker's shared owner is healthy");
    fixture.open_exec();
    let alloc = AllocationId::new("gti-held-release").expect("valid alloc id");
    let action = Action::StartAllocation {
        alloc_id: alloc.clone(),
        workload_id: WorkloadId::new("svc-gti-held-release").expect("valid workload id"),
        node_id: NodeId::new("node-001").expect("valid node id"),
        spec: build_spec(&alloc),
        kind: WorkloadKind::Service,
    };

    let task = {
        let fixture = Arc::clone(&fixture);
        tokio::spawn(async move { fixture.dispatch(action).await })
    };
    tokio::time::timeout(Duration::from_secs(10), driver.release_entered.acquire())
        .await
        .expect("dispatch reaches the post-activation async release")
        .expect("release-entered semaphore remains open")
        .forget();
    assert!(!task.is_finished(), "dispatch remains pending inside the held release future");
    assert_eq!(
        steps_for(&journal, &alloc),
        [Step::Provision, Step::InterceptInstalled, Step::InstallSuccessEvent, Step::Activate],
        "provision, protection, the protection-live event, and activation precede the release"
    );
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

    fixture.worker.stop_alloc(&alloc).await.expect("allocation protection teardown succeeds");
    fixture.worker.shutdown_owner().await.expect("the shared owner shuts down");
}

/// Drive one `StartAllocation` whose activation the owner refuses with a typed
/// post-set-up mismatch, over the seam fixture.
async fn drive_refused_activation(
    alloc_name: &str,
) -> (SeamFixture, Journal, tracing::subscriber::DefaultGuard, AllocationId) {
    let (fixture, journal, trace_guard) =
        journaled_fixture(Arc::new(SimSharedGuestNetworkOwner::default()), true).await;
    fixture.worker.start_shared_owner().await.expect("the worker's shared owner is healthy");
    fixture.open_exec();
    let alloc = AllocationId::new(alloc_name).expect("valid alloc id");
    let action = Action::StartAllocation {
        alloc_id: alloc.clone(),
        workload_id: WorkloadId::new(&format!("svc-{alloc_name}")).expect("workload id"),
        node_id: NodeId::new("node-001").expect("node id"),
        spec: build_spec(&alloc),
        kind: WorkloadKind::Service,
    };
    fixture.dispatch(action).await.expect("the activation refusal is durably projected as Failed");
    (fixture, journal, trace_guard, alloc)
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-52 — The action shim raises the TAP after the protection-live event and before the command
/// CONTRACT_SHAPE: bounded-change.
///
/// The RETAINED activation-refusal projection (active at `3cc00933`): the
/// owner sees provision, then the refused activation, then the structural
/// teardown; the driver starts and stops the allocation once each; EXEC is
/// never released and the running hook never fires; protection is stopped
/// once; and a dominating Failed row names the activation stage. The lease
/// events and the EXEC-gate claim are the sibling body pending 06-04.
#[tokio::test]
async fn tap_activation_failure_stops_vmm_cleans_mtls_and_network_and_dominates_running() {
    let (fixture, journal, _trace_guard, alloc) =
        drive_refused_activation("gti-activation-refused").await;

    let steps = steps_for(&journal, &alloc);
    let owner_steps: Vec<Step> = steps
        .iter()
        .copied()
        .filter(|step| matches!(step, Step::Provision | Step::Activate | Step::Teardown))
        .collect();
    assert_eq!(
        owner_steps,
        [Step::Provision, Step::Activate, Step::Teardown],
        "activation refusal cleanup is provision -> activate -> structural teardown"
    );
    for once in [Step::DriverStart, Step::DriverStop] {
        assert_eq!(
            steps.iter().filter(|step| **step == once).count(),
            1,
            "the driver sees exactly one {once:?}; journal {steps:?}"
        );
    }
    assert!(!steps.contains(&Step::ReleaseForExitEmission), "EXEC is never released: {steps:?}");
    assert!(!steps.contains(&Step::OnAllocRunning), "the running hook never fires: {steps:?}");
    // The three orderings the body asserted at `3cc00933` from inside the
    // owner double: the protection-live event precedes activation, and both
    // the VMM stop and the protection stop precede the structural teardown.
    let at = |step: Step| {
        position(&steps, step).unwrap_or_else(|| panic!("the journal holds {step:?}: {steps:?}"))
    };
    assert!(
        at(Step::InstallSuccessEvent) < at(Step::Activate),
        "the protection-live event precedes activation: {steps:?}"
    );
    assert!(
        at(Step::DriverStop) < at(Step::Teardown),
        "the VMM is stopped before the structural teardown: {steps:?}"
    );
    assert!(
        at(Step::ElementRelease) < at(Step::Teardown),
        "protection is stopped before the structural teardown: {steps:?}"
    );
    assert_eq!(fixture.worker.stop_alloc_calls_for_test(), 1, "one protection stop");
    let row = fixture
        .obs
        .alloc_status_row(&alloc)
        .await
        .expect("activation-failure row read succeeds")
        .expect("activation-failure row is retained");
    assert_eq!(row.state, AllocState::Failed, "the Failed row dominates Running");
    assert!(
        matches!(
            row.reason,
            Some(TransitionReason::WorkloadNetnsProvisionFailed { ref stage, .. })
                if stage == "guest_network_activate"
        ),
        "the Failed row names the activation stage; got {:?}",
        row.reason
    );
    fixture.worker.shutdown_owner().await.expect("the shared owner shuts down");
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-52 — The action shim raises the TAP after the protection-live event and before the command
/// CONTRACT_SHAPE: bounded-change.
///
/// A genuine activation failure takes the full failure projection with its
/// lease events: `driver.stop`, the lease retired, protection stopped, the
/// teardown, and the lease released last, then a dominating Failed row
/// carrying the typed refusal the owner port returned.
#[tokio::test]
#[ignore = "pending DELIVER step 06-04 (S-ND295-52)"]
async fn tap_activation_failure_retires_the_lease_and_releases_it_last() {
    let (fixture, journal, _trace_guard, alloc) =
        drive_refused_activation("gti-activation-refused-lease").await;

    assert_activation_failure_projection("typed activation error", &fixture, &journal, &alloc)
        .await;
    let refusals = fixture.owner.refusals();
    let [refusal] = refusals.as_slice() else {
        panic!("exactly one activation refusal reached the port: {refusals:?}");
    };
    assert_projection_carries_the_refusal("typed activation error", &fixture, &alloc, refusal)
        .await;
    fixture.worker.shutdown_owner().await.expect("the shared owner shuts down");
}

/// The Failed row is `WorkloadNetnsProvisionFailed { stage, detail }` naming
/// the activation stage, after exactly the refusal the owner port returned.
/// The `detail` wording is not pinned (FD § "[REF] Driven port — TAP activation
/// gate (D-295-R5) — ACCEPTED 2026-09-24" (the activation failure projection:
/// "the existing `WorkloadNetnsProvisionFailed { stage:
/// \"guest_network_activate\", detail }`")), so nothing about it is asserted
/// (DISTILL review DR-18).
async fn assert_projection_carries_the_refusal(
    scenario: &str,
    fixture: &SeamFixture,
    alloc: &AllocationId,
    refusal: &ActivationRefusal,
) {
    let row = fixture
        .obs
        .alloc_status_row(alloc)
        .await
        .expect("row read succeeds")
        .expect("the Failed row is retained");
    assert_eq!(&refusal.alloc, alloc, "{scenario}: the refusal the port returned names {alloc}");
    match &row.reason {
        Some(TransitionReason::WorkloadNetnsProvisionFailed { stage, .. }) => {
            assert_eq!(stage, "guest_network_activate", "{scenario}: the activation stage");
        }
        other => panic!(
            "{scenario}: the Failed row is WorkloadNetnsProvisionFailed \
             {{ stage: \"guest_network_activate\", .. }}; got {other:?}"
        ),
    }
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-52 — The action shim raises the TAP after the protection-live event and before the command
/// CONTRACT_SHAPE: bounded-change.
///
/// The owner condemns an allocation after its provision and before its
/// activation reaches the owner: an audit reports the allocation's parts
/// damaged, which condemns it without latching quiescence (FD § "Public deterministic shared-owner simulation API" (the
/// pending condemned-set and `activate` rules)). The activation, held in
/// flight at the owner port until the audit has run, is refused with the exact
/// source-less `PostconditionMismatch { operation: TapObserve, expected: Tap
/// { name: <A's TAP>, ifindex: None, link_kind: Tap, persistent: true, up:
/// false, owner_uid: Some(0) }, observed: None }` (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the
/// outcomes table: a Condemned allocation and a missing allocation record)),
/// and the same failure projection follows, carrying that typed cause.
#[tokio::test]
#[ignore = "pending DELIVER step 06-04 (S-ND295-52)"]
async fn activation_of_a_condemned_allocation_takes_the_failure_projection() {
    let sim_owner = Arc::new(SimSharedGuestNetworkOwner::default());
    let (fixture, journal, _trace_guard) = journaled_fixture(Arc::clone(&sim_owner), false).await;
    let fixture = Arc::new(fixture);
    fixture.worker.start_shared_owner().await.expect("the worker's shared owner is healthy");
    fixture.open_exec();
    let alloc = AllocationId::new("gti-activation-condemned").expect("valid alloc id");
    let action = Action::StartAllocation {
        alloc_id: alloc.clone(),
        workload_id: WorkloadId::new("svc-gti-activation-condemned").expect("workload id"),
        node_id: NodeId::new("node-001").expect("node id"),
        spec: build_spec(&alloc),
        kind: WorkloadKind::Service,
    };
    let hold = fixture.owner.hold_next_activation();
    let task = {
        let fixture = Arc::clone(&fixture);
        tokio::spawn(async move { fixture.dispatch(action).await })
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        while fixture.owner.activations_held() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the provisioned allocation's activation reaches the owner port");
    assert!(
        journal_has(&journal, &alloc, Step::Provision),
        "the allocation is provisioned before the audit condemns it"
    );

    sim_owner.script_audit_damage(std::collections::BTreeSet::from([alloc.clone()]));
    let audit = fixture.owner.audit_shared().await.expect("no node-level component fails");
    assert_eq!(
        audit.damaged.keys().collect::<Vec<_>>(),
        [&alloc],
        "the one audit condemns exactly the provisioned allocation"
    );
    hold.add_permits(1);
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("the held activation completes")
        .expect("the dispatch task joins")
        .expect("the condemned activation is durably projected as Failed");

    assert_activation_failure_projection("condemned allocation", &fixture, &journal, &alloc).await;
    let refusals = fixture.owner.refusals();
    let [refusal] = refusals.as_slice() else {
        panic!("exactly one activation refusal reached the port: {refusals:?}");
    };
    assert_eq!(refusal.alloc, alloc, "the refusal names the condemned allocation");
    // The pinned outcome is "the same source-less `PostconditionMismatch` as a
    // missing allocation record": its operation and expected fact are pinned;
    // `observed` is not, so it is not asserted (DISTILL review DR-18).
    let (operation, expected) = match &refusal.mismatch {
        Some((operation, expected, _observed)) => (*operation, expected.clone()),
        None => panic!(
            "a condemned allocation's activation is refused with a source-less \
             PostconditionMismatch; got {refusal:?}"
        ),
    };
    assert_eq!(
        (operation, expected),
        (
            GuestNetworkOperation::TapObserve,
            GuestNetworkFact::Tap {
                name: refusal.tap.clone(),
                ifindex: None,
                link_kind: GuestLinkKind::Tap,
                persistent: true,
                up: false,
                owner_uid: Some(0),
            },
        ),
        "a condemned allocation's activation is refused with the source-less missing-record \
         mismatch naming its TAP"
    );
    assert_projection_carries_the_refusal("condemned allocation", &fixture, &alloc, refusal).await;
    fixture.worker.shutdown_owner().await.expect("the shared owner shuts down");
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
    let identity = overdrive_control_plane::identity_mgr::IdentityMgr::new(None);
    let result = dispatch_over_network_provisioner(
        vec![Action::RestartAllocation {
            alloc_id: predecessor.clone(),
            spec: build_spec(&successor),
            kind: WorkloadKind::Service,
        }],
        &drivers,
        &alloc_drivers,
        obs.as_ref(),
        &identity,
        store,
        &worker,
        &net_slots,
        &network,
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

// ---------------------------------------------------------------------------
// Restart-abort cleanup detail (S-ND295-54; DISTILL review DR-06, user
// decision 2026-09-29): a restart whose successor start is rejected and whose
// successor cleanup's `stop_alloc` fails persists a Failed row whose detail
// keeps every cause of that stop failure. The stop failure enters at the
// lifecycle driven port through a test-local `MtlsInterceptLifecycle`.
// ---------------------------------------------------------------------------

/// The successor stop failure a [`StopFaultLifecycle`] returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SuccessorStopFault {
    /// Two enforced-handle teardowns failed.
    HandleTeardown,
    /// Every handle teardown succeeded and the element removal failed.
    ElementRemoval,
}

/// The stop error `fault` names for `successor`: two handle teardowns failing
/// with distinct causes, or one element removal failing.
fn successor_stop_error(
    successor: &AllocationId,
    fault: SuccessorStopFault,
) -> MtlsInterceptStopError {
    match fault {
        SuccessorStopFault::HandleTeardown => {
            let first = EnforcedConnectionId::new(successor.clone(), 3);
            MtlsInterceptStopError::HandleTeardown {
                alloc_id: successor.clone(),
                failures: vec![
                    HandleTeardownFailure {
                        connection: first.clone(),
                        source: Arc::new(MtlsEnforcementError::TeardownFailed {
                            id: first,
                            source: std::io::Error::from_raw_os_error(libc::EBADF),
                        }),
                    },
                    HandleTeardownFailure {
                        connection: EnforcedConnectionId::new(successor.clone(), 7),
                        source: Arc::new(MtlsEnforcementError::Io(
                            std::io::Error::from_raw_os_error(libc::ENOTCONN),
                        )),
                    },
                ],
            }
        }
        SuccessorStopFault::ElementRemoval => MtlsInterceptStopError::ElementRemoval {
            alloc_id: successor.clone(),
            source: Arc::new(InterceptError::NftElementUpdateFailed {
                set: InterceptSet::ManagedGuestIps,
                operation: InterceptElementOperation::Delete,
                key: InterceptElementKey::Address(Ipv4Addr::new(100, 95, 0, 7)),
                source: NetlinkError::nft(
                    "shared-element-remove",
                    std::io::Error::from_raw_os_error(libc::EBUSY),
                ),
            }),
        },
    }
}

/// The text FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (`Display`, the text every string consumer carries) pins
/// for `error`, written out from the pin rather than read from the error's own
/// `Display`: `allocation <alloc_id>: enforced-handle teardown failed for <n>
/// handle(s): <c1>: <e1>; <c2>: <e2>` and `allocation <alloc_id>: shared
/// intercept element removal failed: <e>`.
fn pinned_stop_error_text(error: &MtlsInterceptStopError) -> String {
    match error {
        MtlsInterceptStopError::HandleTeardown { alloc_id, failures } => {
            let entries: Vec<String> = failures
                .iter()
                .map(|failure| format!("{}: {}", failure.connection, failure.source))
                .collect();
            format!(
                "allocation {alloc_id}: enforced-handle teardown failed for {} handle(s): {}",
                failures.len(),
                entries.join("; ")
            )
        }
        MtlsInterceptStopError::ElementRemoval { alloc_id, source } => {
            format!("allocation {alloc_id}: shared intercept element removal failed: {source}")
        }
    }
}

/// Test-local `MtlsInterceptLifecycle` (a driven-port fault): `start_alloc`
/// succeeds; `stop_alloc` of the one failing allocation returns a clone of the
/// scripted stop error, and every other stop succeeds. Every stop is recorded.
struct StopFaultLifecycle {
    failing: AllocationId,
    error: MtlsInterceptStopError,
    stops: parking_lot::Mutex<Vec<AllocationId>>,
}

#[async_trait::async_trait]
impl MtlsInterceptLifecycle for StopFaultLifecycle {
    async fn start_alloc(&self, _spec: &AllocationSpec) -> Result<(), MtlsInterceptInstallError> {
        Ok(())
    }

    async fn stop_alloc(&self, alloc_id: &AllocationId) -> Result<(), MtlsInterceptStopError> {
        self.stops.lock().push(alloc_id.clone());
        if *alloc_id == self.failing {
            return Err(self.error.clone());
        }
        Ok(())
    }
}

struct FailedSuccessorStopOutcome {
    result: Result<(), ShimError>,
    successor: AllocationId,
    successor_row: Option<AllocStatusRow>,
    stops: Vec<AllocationId>,
    error: MtlsInterceptStopError,
}

/// Drive a restart whose successor start is rejected
/// (`DriverError::StartRejected`) and whose successor cleanup's `stop_alloc`
/// fails with `fault`, through the one dispatch helper.
async fn drive_restart_abort_with_failed_successor_stop(
    fault: SuccessorStopFault,
) -> FailedSuccessorStopOutcome {
    let tmp = TempDir::new().expect("tempdir");
    let store: Arc<dyn overdrive_core::traits::intent_store::IntentStore> =
        Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open store"));
    let obs = build_obs();
    let stem = format!("restart-stop-fault-{fault:?}").to_ascii_lowercase();
    let predecessor = AllocationId::new(&format!("{stem}-0")).expect("valid predecessor alloc id");
    let successor = AllocationId::new(&format!("{stem}-1")).expect("valid successor alloc id");
    let workload = WorkloadId::new("svc-restart-stop-fault").expect("valid workload id");
    let node = NodeId::new("node-001").expect("valid node id");
    seed_restart_predecessor(obs.as_ref(), &predecessor, &workload, &node).await;
    let driver = Arc::new(RestartAbortDriver {
        scenario: RestartAbortScenario::DriverStart,
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
    net_slots.assign(predecessor.clone()).expect("restart predecessor owns a network slot");
    let network = CountingNetwork::new();
    let error = successor_stop_error(&successor, fault);
    let lifecycle = StopFaultLifecycle {
        failing: successor.clone(),
        error: error.clone(),
        stops: parking_lot::Mutex::new(Vec::new()),
    };
    let identity = overdrive_control_plane::identity_mgr::IdentityMgr::new(None);
    let result = dispatch_over_network_provisioner(
        vec![Action::RestartAllocation {
            alloc_id: predecessor,
            spec: build_spec(&successor),
            kind: WorkloadKind::Service,
        }],
        &drivers,
        &alloc_drivers,
        obs.as_ref(),
        &identity,
        store,
        &lifecycle,
        &net_slots,
        &network,
    )
    .await;
    FailedSuccessorStopOutcome {
        result,
        successor_row: obs.alloc_status_row(&successor).await.expect("successor row read"),
        successor,
        stops: lifecycle.stops.lock().clone(),
        error,
    }
}

/// The persisted Failed row of the rejected successor, and its detail.
///
/// The restart's result follows the shim's existing restart precedence: the
/// Failed row is written first, and the successor's failed cleanup is then the
/// action's error (`successor_abort_cleanup.map_or(Ok(()), Err)` handed to
/// `finish_restart`). The body therefore reads the row whatever the result is,
/// and asserts only that the result carries the scripted stop error — the
/// stop error is not absorbed.
fn failed_successor_detail(outcome: &FailedSuccessorStopOutcome) -> &str {
    match &outcome.result {
        Err(ShimError::MtlsStop(stop)) => {
            assert_eq!(
                stop.to_string(),
                outcome.error.to_string(),
                "the restart surfaces the scripted successor stop error unchanged"
            );
        }
        other => panic!(
            "the restart surfaces the successor's failed cleanup as its error, got {other:?}"
        ),
    }
    assert!(
        outcome.stops.contains(&outcome.successor),
        "the successor cleanup ran its stop_alloc: stops {:?}",
        outcome.stops
    );
    let row = outcome.successor_row.as_ref().expect("the rejected successor has a Failed row");
    assert_eq!(row.state, AllocState::Failed, "the rejected successor's row is Failed: {row:?}");
    let detail =
        row.detail.as_deref().expect("the Failed row carries the DriverStartFailure detail");
    assert!(
        detail.contains("injected restart driver-start rejection"),
        "the primary start rejection stays in the detail: {detail}"
    );
    detail
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-54 — Protection removal is convergent and its failures are typed
/// CONTRACT_SHAPE: bounded-change.
///
/// A restart whose successor start is rejected and whose successor cleanup's
/// `stop_alloc` returns `HandleTeardown` with two failed handles persists a
/// Failed row whose `DriverStartFailure.detail` names both: each
/// `<alloc>#<counter>: <cause>` entry, the `for 2 handle(s)` count, and the
/// whole pinned rendering (DR-06; the pinned `MtlsInterceptStopError`
/// `Display`, which `restart_abort_cleanup_detail` carries unchanged).
#[tokio::test]
#[ignore = "pending DELIVER step 07-01 (S-ND295-54)"]
async fn restart_abort_detail_names_every_failed_handle_teardown_cause() {
    let outcome =
        drive_restart_abort_with_failed_successor_stop(SuccessorStopFault::HandleTeardown).await;
    let detail = failed_successor_detail(&outcome);
    let MtlsInterceptStopError::HandleTeardown { failures, .. } = &outcome.error else {
        panic!("the scripted stop error is HandleTeardown: {:?}", outcome.error);
    };
    assert!(detail.contains("for 2 handle(s)"), "the detail counts both failed handles: {detail}");
    for failure in failures {
        let entry = format!("{}: {}", failure.connection, failure.source);
        assert!(detail.contains(&entry), "the detail names `{entry}`: {detail}");
    }
    let pinned = pinned_stop_error_text(&outcome.error);
    assert!(detail.contains(&pinned), "the detail carries the pinned `{pinned}`: {detail}");
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-54 — Protection removal is convergent and its failures are typed
/// CONTRACT_SHAPE: bounded-change.
///
/// A restart whose successor start is rejected and whose successor cleanup's
/// `stop_alloc` returns `ElementRemoval` persists a Failed row whose
/// `DriverStartFailure.detail` carries `shared intercept element removal
/// failed: <the InterceptError's Display>` (DR-06, ElementRemoval naming its
/// cause, user-approved 2026-09-30).
#[tokio::test]
#[ignore = "pending DELIVER step 07-01 (S-ND295-54)"]
async fn restart_abort_detail_names_the_element_removal_cause() {
    let outcome =
        drive_restart_abort_with_failed_successor_stop(SuccessorStopFault::ElementRemoval).await;
    let detail = failed_successor_detail(&outcome);
    let MtlsInterceptStopError::ElementRemoval { source, .. } = &outcome.error else {
        panic!("the scripted stop error is ElementRemoval: {:?}", outcome.error);
    };
    let cause = format!("shared intercept element removal failed: {source}");
    assert!(detail.contains(&cause), "the detail names the removal cause `{cause}`: {detail}");
    let pinned = pinned_stop_error_text(&outcome.error);
    assert!(detail.contains(&pinned), "the detail carries the pinned `{pinned}`: {detail}");
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
    ) -> overdrive_worker::mtls_intercept::Result<BoundListener> {
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
        source_addr: Ipv4Addr,
        leg_f_port: u16,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        self.inner.install_outbound(source_addr, leg_f_port).map(|guard| self.wrap(guard))
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
            drop(released);
        }
        self.inner.install_inbound(virt, leg_c_port).map(|guard| self.wrap(guard))
    }

    fn observe_shared_state(
        &self,
    ) -> overdrive_worker::mtls_intercept::Result<
        Option<overdrive_worker::mtls_intercept_port::InterceptState>,
    > {
        self.inner.observe_shared_state()
    }

    fn converge_allocation_elements(
        &self,
        expected: &overdrive_worker::mtls_intercept_port::InterceptMembers,
    ) -> overdrive_worker::mtls_intercept::Result<
        Option<overdrive_worker::mtls_intercept_port::InterceptState>,
    > {
        self.inner.converge_allocation_elements(expected)
    }

    fn remove_allocation_elements(
        &self,
        source_addr: Ipv4Addr,
        destinations: &[SocketAddrV4],
    ) -> overdrive_worker::mtls_intercept::Result<
        overdrive_worker::mtls_intercept_port::InterceptState,
    > {
        self.inner.remove_allocation_elements(source_addr, destinations)
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

#[allow(
    clippy::too_many_lines,
    reason = "the driver keeps the complete ordered registration-retired sequence in one body"
)]
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
            let identity = overdrive_control_plane::identity_mgr::IdentityMgr::new(None);
            dispatch_over_network_provisioner(
                vec![action],
                drivers.as_ref(),
                alloc_drivers.as_ref(),
                obs.as_ref(),
                &identity,
                store,
                &worker,
                allocator.as_ref(),
                network.as_ref(),
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
