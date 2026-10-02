//! Acceptance properties for the GH #295 control-plane guest-network contract.
//!
//! The bodies that drive the action owner build `AppState` only through
//! [`SeamFixture`] (test-scenarios § *Seam fixture*): one owner instance that
//! is both `AppState`'s owner and the seam's `provisioner`, one kept
//! `GuestNetworkExecWiring`, one pool from the doc-hidden
//! `GuestAddressPool::new`, and one worker over `SimMtlsEnforcement`,
//! `SimMtlsResolve`, and a recording `MtlsIntercept` that delegates to
//! `SimMtlsIntercept`. The pool's operations are crate-private, so lease state
//! is observed only through the pinned events `guest_network.lease_retired`,
//! `guest_network.lease_released`, and `guest_network.admission_refused`
//! (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the admission refusal projection and the lease events)), captured by [`LeaseEventLayer`] into one ordered
//! [`StepTrace`] together with the driver's stops and the intercept's element
//! removals. Each trace entry samples the sim owner's `calls().len()`, so the
//! owner's calls are interleaved into the same order.

#![allow(clippy::doc_markdown)]

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use overdrive_control_plane::action_shim::{
    ShimError, dispatch_with_guest_network_provisioner_for_test,
};
use overdrive_control_plane::guest_network::{
    GuestAddressPool, GuestNetworkError, GuestNetworkOperation, GuestNetworkPlan,
    GuestNetworkProvisioner, GuestNetworkScratchComplement, GuestNetworkScratchCount,
    SharedGuestNetworkAudit, SharedGuestNetworkAuditError, SharedGuestNetworkOwner, TapActivation,
    TapQuiescence,
};
use overdrive_control_plane::{AppState, noop_heartbeat};
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::guest_network::GuestNetworkExecWiring;
use overdrive_core::id::{AllocationId, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverPayload,
    DriverType, ExitEvent, ExitKind, GuestNetworkAssignment, Resources, VmPayload,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
use overdrive_sim::adapters::mtls_intercept::SimMtlsIntercept;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::{SimIdentityRead, SimMtlsResolve};
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::mtls_intercept::{
    InterceptElementKey, InterceptElementOperation, InterceptError, InterceptPostcondition,
    InterceptSet, NetlinkError,
};
use overdrive_worker::mtls_intercept_port::{
    InterceptGuard, InterceptListener, InterceptMembers, InterceptState, MtlsIntercept,
};
use overdrive_worker::mtls_intercept_worker::{MtlsInterceptStopError, MtlsInterceptWorker};
use parking_lot::Mutex;
use proptest::prelude::*;
use tempfile::TempDir;
use tokio::sync::broadcast::error::TryRecvError;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

fn scratch_complement(counts: &[Option<u32>]) -> GuestNetworkScratchComplement {
    assert_eq!(counts.len(), 15);
    let count = |index: usize| {
        counts[index]
            .map_or(GuestNetworkScratchCount::Unavailable, GuestNetworkScratchCount::Observed)
    };
    GuestNetworkScratchComplement {
        bridges: count(0),
        taps: count(1),
        endpoint_maps: count(2),
        counter_maps: count(3),
        endpoint_entries: count(4),
        tcx_programs: count(5),
        tcx_links: count(6),
        endpoint_map_pins: count(7),
        counter_map_pins: count(8),
        tcx_link_pins: count(9),
        bridge_guard_tables: count(10),
        bridge_guard_chains: count(11),
        bridge_guard_sets: count(12),
        bridge_guard_rules: count(13),
        bridge_guard_members: count(14),
    }
}

proptest! {
    /// CONTRACT_SHAPE: pure-function.
    /// Outcome anchor: DISCUSS Elevator Pitch.
    #[test]
    fn scratch_complement_never_fabricates_zero(
        counts in prop::collection::vec(prop::option::of(0_u32..4), 15..=15),
    ) {
        let complement = scratch_complement(&counts);
        let all_observed = counts.iter().all(Option::is_some);
        let all_zero = counts.iter().all(|count| *count == Some(0));

        prop_assert_eq!(complement.is_fully_observed(), all_observed);
        prop_assert_eq!(complement.is_empty(), all_zero);
        prop_assert!(!complement.is_empty() || complement.is_fully_observed());
    }
}

// ---------------------------------------------------------------------------
// Ordered step trace (test vocabulary, never production types).
// ---------------------------------------------------------------------------

/// How one `Driver::stop` call returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopOutcome {
    /// `Ok(())`: a live VMM was stopped.
    Stopped,
    /// `Err(NotFound)`: the VMM was already gone.
    NotFound,
    /// Any other driver error.
    Failed,
}

/// How one `remove_allocation_elements` call returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemovalOutcome {
    Removed,
    Refused,
}

/// One observation in the ordered step trace.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// One call the sim owner recorded, in its own order.
    Owner(GuestNetworkOperation),
    /// The test-local owner refused `activate` for this allocation.
    ActivateRefused { alloc: String },
    /// `Driver::stop` returned for this allocation.
    DriverStop { alloc: String, outcome: StopOutcome },
    /// `Driver::release_for_exit_emission`: a guest command was released.
    CommandReleased { alloc: String },
    /// Pinned event `guest_network.lease_retired { alloc }`.
    LeaseRetired { alloc: String },
    /// Pinned event `guest_network.lease_released { alloc }`.
    LeaseReleased { alloc: String },
    /// Pinned event `guest_network.admission_refused { alloc, held, retiring, cap }`.
    AdmissionRefused { alloc: String, held: u64, retiring: u64, cap: u64 },
    /// `MtlsIntercept::remove_allocation_elements` returned.
    ElementRemoval {
        source: Ipv4Addr,
        destinations: BTreeSet<SocketAddrV4>,
        outcome: RemovalOutcome,
    },
}

struct TraceEntry {
    step: Step,
    owner_calls: usize,
}

/// A position in the trace: the entry count and the sim owner's call count.
struct TraceMark {
    entries: usize,
    owner_calls: usize,
}

/// One ordered trace shared by the tracing layer, the driver, the owner, and
/// the intercept of one fixture.
struct StepTrace {
    sim_owner: Arc<SimSharedGuestNetworkOwner>,
    entries: Mutex<Vec<TraceEntry>>,
}

impl StepTrace {
    const fn new(sim_owner: Arc<SimSharedGuestNetworkOwner>) -> Self {
        Self { sim_owner, entries: Mutex::new(Vec::new()) }
    }

    fn record(&self, step: Step) {
        let owner_calls = self.sim_owner.calls().len();
        self.entries.lock().push(TraceEntry { step, owner_calls });
    }

    fn mark(&self) -> TraceMark {
        TraceMark { entries: self.entries.lock().len(), owner_calls: self.sim_owner.calls().len() }
    }

    /// Every step since `mark`, with the sim owner's calls interleaved at the
    /// positions their sampled counts place them.
    fn since(&self, mark: &TraceMark) -> Vec<Step> {
        let calls = self.sim_owner.calls();
        let entries = self.entries.lock();
        let mut steps = Vec::new();
        let mut next_call = mark.owner_calls;
        for entry in entries.iter().skip(mark.entries) {
            while next_call < entry.owner_calls.min(calls.len()) {
                steps.push(Step::Owner(calls[next_call]));
                next_call += 1;
            }
            steps.push(entry.step.clone());
        }
        drop(entries);
        steps.extend(calls.iter().skip(next_call).copied().map(Step::Owner));
        steps
    }
}

/// `steps` without `LeaseRetired { alloc }` entries: FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the lease events) pins that `retire`
/// emits `lease_retired`, but not whether a `retire` of an already-Retiring
/// lease (which changes nothing and returns `false`, FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the pool operations: `retire`)) emits it.
fn without_noop_retire(steps: Vec<Step>, alloc: &str) -> Vec<Step> {
    steps
        .into_iter()
        .filter(|step| !matches!(step, Step::LeaseRetired { alloc: retired } if retired == alloc))
        .collect()
}

// ---------------------------------------------------------------------------
// Test-local tracing layer: the pinned lease events.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct LeaseEventFields {
    alloc: Option<String>,
    held: Option<u64>,
    retiring: Option<u64>,
    cap: Option<u64>,
}

impl LeaseEventFields {
    /// The allocation named by the event, in its `Display` form whether the
    /// field was recorded with `%` or `?`.
    fn alloc(&self) -> String {
        let raw = self.alloc.as_deref().unwrap_or_default();
        raw.strip_prefix("AllocationId(\"")
            .and_then(|inner| inner.strip_suffix("\")"))
            .unwrap_or(raw)
            .to_owned()
    }

    fn store(&mut self, field: &Field, value: String) {
        if field.name() == "alloc" {
            self.alloc = Some(value);
        }
    }

    fn store_count(&mut self, field: &Field, value: u64) {
        match field.name() {
            "held" => self.held = Some(value),
            "retiring" => self.retiring = Some(value),
            "cap" => self.cap = Some(value),
            _ => {}
        }
    }
}

impl Visit for LeaseEventFields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.store(field, format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.store(field, value.to_owned());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.store_count(field, value);
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.store_count(field, u64::try_from(value).unwrap_or(u64::MAX));
    }
}

/// Records the three pinned lease events (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the admission refusal projection and the lease events)) into the trace.
struct LeaseEventLayer {
    trace: Arc<StepTrace>,
}

impl<S: Subscriber> Layer<S> for LeaseEventLayer {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let name = event.metadata().name();
        if !matches!(
            name,
            "guest_network.lease_retired"
                | "guest_network.lease_released"
                | "guest_network.admission_refused"
        ) {
            return;
        }
        let mut fields = LeaseEventFields::default();
        event.record(&mut fields);
        let alloc = fields.alloc();
        let step = match name {
            "guest_network.lease_retired" => Step::LeaseRetired { alloc },
            "guest_network.lease_released" => Step::LeaseReleased { alloc },
            _ => Step::AdmissionRefused {
                alloc,
                held: fields.held.unwrap_or(u64::MAX),
                retiring: fields.retiring.unwrap_or(u64::MAX),
                cap: fields.cap.unwrap_or(u64::MAX),
            },
        };
        self.trace.record(step);
    }
}

// ---------------------------------------------------------------------------
// Test-local driven-port doubles.
// ---------------------------------------------------------------------------

/// `SimDriver` whose stops and command releases enter the trace.
struct TraceDriver {
    inner: SimDriver,
    trace: Arc<StepTrace>,
}

#[async_trait::async_trait]
impl Driver for TraceDriver {
    fn r#type(&self) -> DriverType {
        self.inner.r#type()
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        self.inner.start(spec).await
    }

    async fn release_for_exit_emission(&self, handle: &AllocationHandle) {
        self.trace.record(Step::CommandReleased { alloc: handle.alloc.to_string() });
        self.inner.release_for_exit_emission(handle).await;
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        let result = self.inner.stop(handle).await;
        let outcome = match &result {
            Ok(()) => StopOutcome::Stopped,
            Err(DriverError::NotFound { .. }) => StopOutcome::NotFound,
            Err(_) => StopOutcome::Failed,
        };
        self.trace.record(Step::DriverStop { alloc: handle.alloc.to_string(), outcome });
        result
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

    fn take_exit_receiver(&self) -> Option<tokio::sync::mpsc::Receiver<ExitEvent>> {
        self.inner.take_exit_receiver()
    }

    fn on_alloc_running(&self, spec: &AllocationSpec) {
        self.inner.on_alloc_running(spec);
    }

    fn on_alloc_terminal(&self, alloc_id: &AllocationId) {
        self.inner.on_alloc_terminal(alloc_id);
    }

    fn on_alloc_stable(&self, alloc_id: &AllocationId) {
        self.inner.on_alloc_stable(alloc_id);
    }

    fn live_allocations(&self) -> Option<Vec<AllocationId>> {
        self.inner.live_allocations()
    }

    fn try_begin_reclamation(&self, alloc: &AllocationId) -> bool {
        self.inner.try_begin_reclamation(alloc)
    }

    fn release_supervision(&self, alloc: &AllocationId) {
        self.inner.release_supervision(alloc);
    }
}

/// The listener type `MtlsIntercept::bind_transparent` returns. The DELIVER
/// step that carries B-7 (05-01 at the latest) changes it to
/// `Arc<dyn InterceptListener>` (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the pinned `bind_transparent` signature)); the delegation below is
/// unchanged by that step.
type BoundListener = Arc<dyn InterceptListener>;

/// The `op` of the element-removal failure [`RecordingIntercept`] injects.
const INJECTED_REMOVAL_OP: &str = "nd295-injected-member-delete";

/// The removal failure a rejected delete batch reports (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the `remove_allocation_elements` contract)): the
/// managed-guest member of `source` could not be deleted, `EBUSY`.
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

/// Whether `error` is exactly the failure [`injected_removal_failure`] built
/// for `source`, matched by variant and every field of the cause.
fn is_injected_removal_failure(error: &InterceptError, source: Ipv4Addr) -> bool {
    matches!(
        error,
        InterceptError::NftElementUpdateFailed {
            set: InterceptSet::ManagedGuestIps,
            operation: InterceptElementOperation::Delete,
            key: InterceptElementKey::Address(key),
            source: NetlinkError::Nft { op, source: io, .. },
        } if *key == source
            && *op == INJECTED_REMOVAL_OP
            && io.raw_os_error() == Some(libc::EBUSY)
    )
}

/// `SimMtlsIntercept` whose element removals enter the trace, with one
/// standing removal fault: while armed, a removal is rejected before any
/// effect and the pre-state is preserved.
struct RecordingIntercept {
    inner: SimMtlsIntercept,
    trace: Arc<StepTrace>,
    removal_fault: AtomicBool,
}

impl RecordingIntercept {
    fn new(trace: Arc<StepTrace>) -> Self {
        Self { inner: SimMtlsIntercept::new(), trace, removal_fault: AtomicBool::new(false) }
    }

    fn script_removal_failure(&self, armed: bool) {
        self.removal_fault.store(armed, Ordering::SeqCst);
    }

    /// The members the double's model holds now.
    fn members(&self) -> InterceptMembers {
        self.inner
            .observe_shared_state()
            .expect("the intercept model is observable")
            .expect("the shared program is in the intercept model")
            .members
    }

    /// Another actor deletes `source`'s members and the inbound
    /// `destinations` from the double's model, out of band: the model changes
    /// directly, with no port call of the production path and no trace entry.
    fn remove_members_out_of_band(&self, source: Ipv4Addr, destinations: &BTreeSet<SocketAddrV4>) {
        let mut members = self.members();
        members.managed_guest_ips.remove(&source);
        members.outbound_sources.remove(&source);
        for destination in destinations {
            members.inbound_destinations.remove(destination);
        }
        self.inner
            .converge_allocation_elements(&members)
            .expect("the out-of-band delete lands in the intercept model")
            .expect("the shared program is in the intercept model");
    }
}

impl MtlsIntercept for RecordingIntercept {
    fn bind_transparent(
        &self,
        addr: SocketAddrV4,
    ) -> overdrive_worker::mtls_intercept::Result<BoundListener> {
        self.inner.bind_transparent(addr)
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
        agent_leg_f_port: u16,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        self.inner.install_outbound(source_addr, agent_leg_f_port)
    }

    fn install_inbound(
        &self,
        virt: SocketAddrV4,
        agent_leg_c_port: u16,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        self.inner.install_inbound(virt, agent_leg_c_port)
    }

    fn observe_shared_state(
        &self,
    ) -> overdrive_worker::mtls_intercept::Result<Option<InterceptState>> {
        self.inner.observe_shared_state()
    }

    fn converge_allocation_elements(
        &self,
        expected: &InterceptMembers,
    ) -> overdrive_worker::mtls_intercept::Result<Option<InterceptState>> {
        self.inner.converge_allocation_elements(expected)
    }

    fn remove_allocation_elements(
        &self,
        source_addr: Ipv4Addr,
        destinations: &[SocketAddrV4],
    ) -> overdrive_worker::mtls_intercept::Result<InterceptState> {
        let result = if self.removal_fault.load(Ordering::SeqCst) {
            Err(injected_removal_failure(source_addr))
        } else {
            self.inner.remove_allocation_elements(source_addr, destinations)
        };
        self.trace.record(Step::ElementRemoval {
            source: source_addr,
            destinations: destinations.iter().copied().collect(),
            outcome: if result.is_ok() { RemovalOutcome::Removed } else { RemovalOutcome::Refused },
        });
        result
    }
}

/// The test-local owner of S-ND295-56: it implements both owner ports by
/// delegating to the one `SimSharedGuestNetworkOwner`, except that while its
/// activation refusal is armed `activate` returns a typed error and changes
/// nothing.
struct ActivationFaultOwner {
    inner: Arc<SimSharedGuestNetworkOwner>,
    trace: Arc<StepTrace>,
    refuse_activation: AtomicBool,
    /// The owner's model of attachment parts: the allocations whose parts
    /// exist. A successful provision adds one; a successful teardown, or
    /// another actor out of band, removes it.
    attached: Mutex<BTreeSet<AllocationId>>,
    /// Every successful teardown, with whether the allocation's parts were
    /// present when it ran (`false`: it converged on parts already gone).
    teardowns: Mutex<Vec<(AllocationId, bool)>>,
}

impl ActivationFaultOwner {
    fn script_activation_failure(&self, armed: bool) {
        self.refuse_activation.store(armed, Ordering::SeqCst);
    }

    /// Another actor removes every attachment part of `alloc` out of band.
    /// Returns whether the parts were present.
    fn remove_parts_out_of_band(&self, alloc: &AllocationId) -> bool {
        self.attached.lock().remove(alloc)
    }

    fn teardowns(&self) -> Vec<(AllocationId, bool)> {
        self.teardowns.lock().clone()
    }
}

#[async_trait::async_trait]
impl GuestNetworkProvisioner for ActivationFaultOwner {
    async fn provision(
        &self,
        plan: &GuestNetworkPlan,
    ) -> overdrive_control_plane::guest_network::Result<()> {
        self.inner.provision(plan).await?;
        self.attached.lock().insert(plan.alloc().clone());
        Ok(())
    }

    async fn activate(
        &self,
        plan: &GuestNetworkPlan,
    ) -> overdrive_control_plane::guest_network::Result<TapActivation> {
        if self.refuse_activation.load(Ordering::SeqCst) {
            self.trace.record(Step::ActivateRefused { alloc: plan.alloc().to_string() });
            return Err(GuestNetworkError::Io {
                operation: GuestNetworkOperation::TapSetUp,
                source: std::io::Error::other("scripted activation refusal"),
            });
        }
        self.inner.activate(plan).await
    }

    /// Converges on absence: removes the allocation's parts if present, and
    /// succeeds when they are already gone.
    async fn teardown(
        &self,
        plan: &GuestNetworkPlan,
    ) -> overdrive_control_plane::guest_network::Result<()> {
        self.inner.teardown(plan).await?;
        let present = self.attached.lock().remove(plan.alloc());
        self.teardowns.lock().push((plan.alloc().clone(), present));
        Ok(())
    }
}

#[async_trait::async_trait]
impl SharedGuestNetworkOwner for ActivationFaultOwner {
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

// ---------------------------------------------------------------------------
// Seam fixture (test-scenarios § Seam fixture; FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (how the gate reaches the shim, and the `AppState` constructors); FD § "C-295-B — network provisioner boundary" (the B-1 pin that both test helpers read the gate and the pool from `state`)).
// ---------------------------------------------------------------------------

/// Distinct declared Service ports, as the production service projection
/// hands them to the action owner (deduplicated TCP ports).
const SERVICE_PORTS: [u16; 2] = [8080, 53];

struct SeamFixture<O> {
    state: AppState,
    driver: Arc<TraceDriver>,
    /// The one owner instance: `AppState`'s owner from DELIVER 05-01 and the
    /// seam's `provisioner` on every dispatch.
    owner: Arc<O>,
    /// The sim owner every owner call reaches; its `calls()` are the owner
    /// half of the trace.
    sim_owner: Arc<SimSharedGuestNetworkOwner>,
    wiring: GuestNetworkExecWiring,
    #[allow(dead_code, reason = "passed to AppState by the constructor call from DELIVER 05-01")]
    pool: Arc<GuestAddressPool>,
    worker: Arc<MtlsInterceptWorker>,
    intercept: Arc<RecordingIntercept>,
    trace: Arc<StepTrace>,
    ticks: AtomicU64,
    _trace_guard: tracing::subscriber::DefaultGuard,
    _tmp: TempDir,
}

impl SeamFixture<SimSharedGuestNetworkOwner> {
    /// A fixture whose one owner instance is the `SimSharedGuestNetworkOwner`.
    async fn with_sim_owner() -> Self {
        Self::build(|sim_owner, _trace| Arc::clone(sim_owner)).await
    }
}

impl SeamFixture<ActivationFaultOwner> {
    /// A fixture whose one owner instance is the [`ActivationFaultOwner`].
    async fn with_activation_fault_owner() -> Self {
        Self::build(|sim_owner, trace| {
            Arc::new(ActivationFaultOwner {
                inner: Arc::clone(sim_owner),
                trace: Arc::clone(trace),
                refuse_activation: AtomicBool::new(false),
                attached: Mutex::new(BTreeSet::new()),
                teardowns: Mutex::new(Vec::new()),
            })
        })
        .await
    }
}

impl<O> SeamFixture<O>
where
    O: SharedGuestNetworkOwner + 'static,
{
    async fn build(
        owner_for: impl FnOnce(&Arc<SimSharedGuestNetworkOwner>, &Arc<StepTrace>) -> Arc<O>,
    ) -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let clock = Arc::new(SimClock::new());
        let sim_owner = Arc::new(SimSharedGuestNetworkOwner::default());
        let trace = Arc::new(StepTrace::new(Arc::clone(&sim_owner)));
        let owner = owner_for(&sim_owner, &trace);
        let wiring = GuestNetworkExecWiring::new(Arc::clone(&clock) as Arc<dyn Clock>);
        // Today's pool constants (the process action pool the dispatch path
        // reads until DELIVER 06-03 reads `state.guest_pool`).
        let pool = Arc::new(GuestAddressPool::new(
            ipnet::Ipv4Net::new(Ipv4Addr::new(100, 95, 0, 0), 16).expect("node guest prefix"),
            "ovd-gbr0".to_owned(),
            Ipv4Addr::new(100, 95, 0, 1),
            Ipv4Addr::new(100, 95, 0, 1),
        ));
        let intercept = Arc::new(RecordingIntercept::new(Arc::clone(&trace)));
        let identity: Arc<dyn IdentityRead> =
            Arc::new(SimIdentityRead::new(std::collections::BTreeMap::new(), None));
        let enforcement: Arc<dyn MtlsEnforcement> =
            Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()));
        let resolve: Arc<dyn MtlsResolve> = Arc::new(SimMtlsResolve::new(
            std::collections::BTreeMap::new(),
            MtlsResolution::NonMesh,
        ));
        let worker = Arc::new(MtlsInterceptWorker::new(
            enforcement,
            resolve,
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::clone(&intercept) as Arc<dyn MtlsIntercept>,
        ));
        let driver = Arc::new(TraceDriver {
            inner: SimDriver::new(DriverType::Vm),
            trace: Arc::clone(&trace),
        });

        let mut runtime =
            overdrive_control_plane::reconciler_runtime::ReconcilerRuntime::new_with_redb_view_store_for_test(
                tmp.path(),
            )
            .expect("runtime");
        runtime.register(noop_heartbeat()).await.expect("register reconciler");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("intent store"));
        let obs: Arc<dyn ObservationStore> =
            Arc::new(SimObservationStore::single_peer(node_id(), 0));
        let allocator = overdrive_control_plane::test_default_allocator(
            Arc::clone(&store) as Arc<dyn IntentStore>
        );
        // The constructor call. Until DELIVER 05-01 cuts the pinned
        // constructors (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the `AppState` constructors)) it passes today's inputs; 05-01 appends
        // `Arc::clone(&worker)`, `Arc::clone(&owner) as Arc<dyn SharedGuestNetworkOwner>`,
        // `wiring.gate()`, and `Arc::clone(&pool)`, and nothing else in this
        // file changes with the constructors.
        let state = AppState::new(
            store,
            store_path,
            obs,
            Arc::new(runtime),
            Arc::clone(&driver) as Arc<dyn Driver>,
            clock,
            Arc::new(SimDataplane::new()),
            Arc::new(SimCa::new(Arc::new(SimEntropy::new(295)))),
            Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
            node_id(),
            allocator,
            overdrive_control_plane::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            Arc::clone(&worker),
            Arc::clone(&owner) as Arc<dyn SharedGuestNetworkOwner>,
            wiring.gate(),
            Arc::clone(&pool),
        );
        let trace_guard = tracing::subscriber::set_default(
            tracing_subscriber::registry().with(LeaseEventLayer { trace: Arc::clone(&trace) }),
        );
        Self {
            state,
            driver,
            owner,
            sim_owner,
            wiring,
            pool,
            worker,
            intercept,
            trace,
            ticks: AtomicU64::new(0),
            _trace_guard: trace_guard,
            _tmp: tmp,
        }
    }

    /// Start the worker's shared owner (it binds nothing over the sim port
    /// from B-7, FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the `SimMtlsIntercept` contract)) and open the EXEC gate through the paired
    /// supervisor, as a body whose dispatch reaches intercept install and
    /// `claim_release` must (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (gate state outside `run_server*`)).
    async fn start_protection_and_open_exec(&self) {
        self.worker.start_shared_owner().await.expect("the shared intercept owner starts");
        assert!(
            self.wiring.supervisor().open_after_boot(),
            "the fixture's EXEC gate starts BootClosed and only its supervisor opens it"
        );
    }

    async fn dispatch(&self, action: Action) -> Result<(), ShimError> {
        let counter = self.ticks.fetch_add(1, Ordering::SeqCst) + 1;
        Box::pin(dispatch_with_guest_network_provisioner_for_test(
            vec![action],
            &self.state,
            &tick(counter),
            self.owner.as_ref(),
        ))
        .await
    }

    fn started_assignment(&self, index: usize) -> GuestNetworkAssignment {
        let specs = self.driver.inner.started_specs();
        specs[index].network.clone().expect("canonical grouped assignment")
    }

    async fn row_state(&self, alloc: &str) -> Option<AllocState> {
        self.state
            .obs
            .alloc_status_row(&alloc_id(alloc))
            .await
            .expect("allocation row read succeeds")
            .map(|row| row.state)
    }

    /// Dispatch a reclaim of `alloc` and assert it wrote no allocation row,
    /// recorded no lifecycle occurrence, and emitted no lifecycle event
    /// (FD § "[REF] Lifecycle action — row-neutral reclaim (D-295-R11) — ACCEPTED 2026-09-24" (the shim arm)).
    async fn reclaim_without_row_or_event(&self, alloc: &str) -> Result<(), ShimError> {
        let rows_before = self.state.obs.alloc_status_rows().await.expect("rows readable");
        let occurrences_before = self
            .state
            .obs
            .alloc_lifecycle_occurrences(&alloc_id(alloc))
            .await
            .expect("occurrences readable");
        let mut events = self.state.lifecycle_events.subscribe();

        let result = self.dispatch(reclaim_action(alloc)).await;

        assert_eq!(
            self.state.obs.alloc_status_rows().await.expect("rows readable"),
            rows_before,
            "a reclaim of {alloc} writes no allocation row"
        );
        assert_eq!(
            self.state
                .obs
                .alloc_lifecycle_occurrences(&alloc_id(alloc))
                .await
                .expect("occurrences readable"),
            occurrences_before,
            "a reclaim of {alloc} records no lifecycle occurrence"
        );
        assert!(
            matches!(events.try_recv(), Err(TryRecvError::Empty)),
            "a reclaim of {alloc} emits no lifecycle event"
        );
        result
    }
}

fn node_id() -> NodeId {
    NodeId::new("nd295-node").expect("node id")
}

fn alloc_id(name: &str) -> AllocationId {
    AllocationId::new(name).expect("allocation id")
}

fn spec(name: &str) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc_id(name),
        identity: SpiffeId::new(&format!("spiffe://overdrive.local/workload/nd295/alloc/{name}"))
            .expect("SPIFFE ID"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/bin/true".to_owned(),
            args: Vec::new(),
            kernel: PathBuf::from("/conformance/kernel"),
            rootfs: PathBuf::from("/conformance/rootfs.ext4"),
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        network: None,
        service_ports: SERVICE_PORTS
            .into_iter()
            .map(|port| NonZeroU16::new(port).expect("non-zero listener port"))
            .collect(),
    }
}

fn start_action(name: &str) -> Action {
    Action::StartAllocation {
        alloc_id: alloc_id(name),
        workload_id: WorkloadId::new(&format!("workload-{name}")).expect("workload id"),
        node_id: node_id(),
        spec: spec(name),
        kind: WorkloadKind::Service,
    }
}

fn stop_action(name: &str) -> Action {
    Action::StopAllocation {
        alloc_id: alloc_id(name),
        terminal: Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
    }
}

fn reclaim_action(name: &str) -> Action {
    Action::ReclaimAllocationNetwork { alloc_id: alloc_id(name) }
}

/// The inbound destinations an allocation at `address` declares.
fn destinations(address: Ipv4Addr) -> BTreeSet<SocketAddrV4> {
    SERVICE_PORTS.into_iter().map(|port| SocketAddrV4::new(address, port)).collect()
}

fn tick(counter: u64) -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(counter)),
        tick: counter,
        deadline: now + Duration::from_secs(1),
    }
}

// ---------------------------------------------------------------------------
// S-ND295-06
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-SHARED-SWITCH
/// S-ND295-06 — Start publishes no partial attachment
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 06-03 (S-ND295-06)"]
async fn provision_refusal_stops_before_driver_start_and_preserves_the_typed_owner_cause() {
    const REFUSED: &str = "nd295-provision-refused";
    let fixture = SeamFixture::with_sim_owner().await;
    fixture.sim_owner.script_provision_failure(true);
    let start = fixture.trace.mark();

    let error = fixture
        .dispatch(start_action(REFUSED))
        .await
        .expect_err("typed provision refusal reaches the production action owner");

    assert!(
        matches!(
            error,
            ShimError::GuestNetwork(GuestNetworkError::Io {
                operation: GuestNetworkOperation::TapCreate,
                ..
            })
        ),
        "the owner's typed provision cause is the dispatch error, got {error:?}"
    );
    assert!(fixture.driver.inner.started_specs().is_empty(), "the VMM driver is never entered");
    assert_eq!(fixture.driver.inner.live_count(), 0);
    // A stop that finds no VMM changes nothing; every other step of the start
    // is compared whole, so a released command, a stopped VMM, an element
    // removal, or any extra owner call or lease event is a failure.
    let steps: Vec<Step> = fixture
        .trace
        .since(&start)
        .into_iter()
        .filter(|step| !matches!(step, Step::DriverStop { outcome: StopOutcome::NotFound, .. }))
        .collect();
    assert_eq!(
        steps,
        vec![
            Step::Owner(GuestNetworkOperation::TapCreate),
            Step::LeaseRetired { alloc: REFUSED.to_owned() },
            Step::Owner(GuestNetworkOperation::TapDelete),
            Step::LeaseReleased { alloc: REFUSED.to_owned() },
        ],
        "the lease is retired before the owner's teardown and released only after it succeeds"
    );
}

// ---------------------------------------------------------------------------
// S-ND295-07
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-SHARED-SWITCH
/// S-ND295-07 — Stop removes the predecessor completely before its address is reused
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 07-01 (S-ND295-07)"]
async fn teardown_failure_holds_the_lease_until_retry_completes_then_allows_exact_address_reuse() {
    const PREDECESSOR: &str = "nd295-predecessor";
    let fixture = SeamFixture::with_sim_owner().await;
    fixture.start_protection_and_open_exec().await;

    fixture
        .dispatch(start_action(PREDECESSOR))
        .await
        .expect("predecessor starts with one grouped network assignment");
    let predecessor = fixture.started_assignment(0);

    // The first stop: the VMM is confirmed gone, the lease retired, the
    // protection removed, and the owner's teardown refused, so the lease is
    // never released.
    fixture.sim_owner.script_teardown_failure(true);
    let failed_stop = fixture.trace.mark();
    let stop_error = fixture
        .dispatch(stop_action(PREDECESSOR))
        .await
        .expect_err("effect-first teardown refusal retains the lease");
    assert!(
        matches!(
            stop_error,
            ShimError::GuestNetwork(GuestNetworkError::Io {
                operation: GuestNetworkOperation::TapDelete,
                ..
            })
        ),
        "the owner's typed teardown cause is the dispatch error, got {stop_error:?}"
    );
    assert_eq!(
        fixture.trace.since(&failed_stop),
        vec![
            Step::DriverStop { alloc: PREDECESSOR.to_owned(), outcome: StopOutcome::Stopped },
            Step::LeaseRetired { alloc: PREDECESSOR.to_owned() },
            Step::ElementRemoval {
                source: predecessor.address,
                destinations: destinations(predecessor.address),
                outcome: RemovalOutcome::Removed,
            },
            Step::Owner(GuestNetworkOperation::TapDelete),
        ],
        "driver.stop → lease_retired → element removal → teardown, and no lease_released"
    );

    // While the predecessor's lease is held, an unrelated start receives a
    // different address.
    let while_held_mark = fixture.trace.mark();
    fixture
        .dispatch(start_action("nd295-while-held"))
        .await
        .expect("unrelated allocation remains startable");
    let while_held = fixture.started_assignment(1);
    assert_ne!(while_held.address, predecessor.address);
    assert!(
        !fixture
            .trace
            .since(&while_held_mark)
            .contains(&Step::LeaseReleased { alloc: PREDECESSOR.to_owned() }),
        "the predecessor's lease stays held while its teardown is incomplete"
    );

    // The retried stop completes the teardown and only then releases the
    // lease. The protection was already removed, so no removal repeats
    // (B-6 caller rule 2, FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (what each caller receives, rule 2)), and the VMM stays gone.
    fixture.sim_owner.script_teardown_failure(false);
    let retry = fixture.trace.mark();
    fixture
        .dispatch(stop_action(PREDECESSOR))
        .await
        .expect("retry completes the retained teardown before release");
    assert_eq!(
        without_noop_retire(fixture.trace.since(&retry), PREDECESSOR),
        vec![
            Step::DriverStop { alloc: PREDECESSOR.to_owned(), outcome: StopOutcome::NotFound },
            Step::Owner(GuestNetworkOperation::TapDelete),
            Step::LeaseReleased { alloc: PREDECESSOR.to_owned() },
        ],
        "teardown → lease_released, released last"
    );

    fixture
        .dispatch(start_action("nd295-successor"))
        .await
        .expect("successor starts after predecessor complement is empty");
    let successor = fixture.started_assignment(2);
    assert_eq!(successor.address, predecessor.address);
    assert_eq!(successor.tap, predecessor.tap);
    assert_eq!(successor.mac, predecessor.mac);
    assert_eq!(
        fixture.sim_owner.calls(),
        [
            GuestNetworkOperation::TapCreate,
            GuestNetworkOperation::TapSetUp,
            GuestNetworkOperation::TapDelete,
            GuestNetworkOperation::TapCreate,
            GuestNetworkOperation::TapSetUp,
            GuestNetworkOperation::TapDelete,
            GuestNetworkOperation::TapCreate,
            GuestNetworkOperation::TapSetUp,
        ],
        "each successful start activates exactly once before release; teardown/retry remains effect-first"
    );
}

// ---------------------------------------------------------------------------
// S-ND295-56
// ---------------------------------------------------------------------------

impl SeamFixture<ActivationFaultOwner> {
    /// Precondition through the production path (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the activation failure projection)): `alloc`
    /// starts, its activation is refused, and its protection removal fails, so
    /// the action owner confirms the VMM gone, retires the lease, withholds
    /// teardown and release, and writes the allocation's Failed row. The
    /// worker keeps the allocation's retirement for a retry (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the worker's retirement outcomes)).
    async fn finished_allocation_with_a_retiring_lease(&self, alloc: &str) -> Ipv4Addr {
        self.start_protection_and_open_exec().await;
        self.owner.script_activation_failure(true);
        self.intercept.script_removal_failure(true);
        let start = self.trace.mark();
        // The start's own result is the activation projection's (S-ND295-52);
        // this precondition requires only the state it leaves.
        let started = self.dispatch(start_action(alloc)).await;
        let steps = self.trace.since(&start);
        assert_eq!(
            self.row_state(alloc).await,
            Some(AllocState::Failed),
            "precondition: the refused activation writes the allocation's Failed row \
             (start result {started:?}, steps {steps:#?})"
        );
        assert!(
            steps.contains(&Step::LeaseRetired { alloc: alloc.to_owned() })
                && !steps.contains(&Step::LeaseReleased { alloc: alloc.to_owned() }),
            "precondition: the lease is retired and still held (steps {steps:#?})"
        );
        self.owner.script_activation_failure(false);
        self.intercept.script_removal_failure(false);
        self.started_assignment(0).address
    }
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH
/// S-ND295-56 — Reclaim cleans an allocation's network without touching its row
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 07-02 (S-ND295-56)"]
async fn reclaim_cleans_a_leased_finished_allocation_without_a_row() {
    const FINISHED: &str = "nd295-reclaim-finished";
    let fixture = SeamFixture::with_activation_fault_owner().await;
    let address = fixture.finished_allocation_with_a_retiring_lease(FINISHED).await;

    let reclaim = fixture.trace.mark();
    fixture
        .reclaim_without_row_or_event(FINISHED)
        .await
        .expect("the reclaim of a finished, leased allocation completes");

    assert_eq!(
        without_noop_retire(fixture.trace.since(&reclaim), FINISHED),
        vec![
            Step::DriverStop { alloc: FINISHED.to_owned(), outcome: StopOutcome::NotFound },
            Step::ElementRemoval {
                source: address,
                destinations: destinations(address),
                outcome: RemovalOutcome::Removed,
            },
            Step::Owner(GuestNetworkOperation::TapDelete),
            Step::LeaseReleased { alloc: FINISHED.to_owned() },
        ],
        "retire → VMM confirmed gone → element removal → teardown → lease_released"
    );
    assert_eq!(fixture.row_state(FINISHED).await, Some(AllocState::Failed));
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH
/// S-ND295-56 — Reclaim cleans an allocation's network without touching its row
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 07-02 (S-ND295-56)"]
async fn reclaim_without_a_lease_does_nothing() {
    const NEVER_LEASED: &str = "nd295-reclaim-never-leased";
    const RELEASED: &str = "nd295-reclaim-released";
    let fixture = SeamFixture::with_activation_fault_owner().await;
    fixture.start_protection_and_open_exec().await;

    // An allocation this node never leased.
    let never_leased = fixture.trace.mark();
    fixture
        .reclaim_without_row_or_event(NEVER_LEASED)
        .await
        .expect("a reclaim without a lease returns Ok");
    assert_eq!(
        fixture.trace.since(&never_leased),
        Vec::<Step>::new(),
        "no owner, driver, or intercept call and no lease event"
    );

    // An allocation whose stop completed and released its lease.
    fixture.dispatch(start_action(RELEASED)).await.expect("allocation starts");
    let stop = fixture.trace.mark();
    fixture.dispatch(stop_action(RELEASED)).await.expect("allocation stops completely");
    assert!(
        fixture.trace.since(&stop).contains(&Step::LeaseReleased { alloc: RELEASED.to_owned() }),
        "precondition: the stop released the lease"
    );
    let released = fixture.trace.mark();
    fixture
        .reclaim_without_row_or_event(RELEASED)
        .await
        .expect("a reclaim after the lease was released returns Ok");
    assert_eq!(
        fixture.trace.since(&released),
        Vec::<Step>::new(),
        "no owner, driver, or intercept call and no lease event"
    );
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH
/// S-ND295-56 — Reclaim cleans an allocation's network without touching its row
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 07-02 (S-ND295-56)"]
async fn a_failed_reclaim_step_keeps_the_lease_for_the_next_attempt() {
    const FINISHED: &str = "nd295-reclaim-failing";
    let fixture = SeamFixture::with_activation_fault_owner().await;
    let address = fixture.finished_allocation_with_a_retiring_lease(FINISHED).await;
    let finished = || FINISHED.to_owned();

    // 1. Protection removal fails: its typed cause is returned and every later
    //    effect is withheld.
    fixture.intercept.script_removal_failure(true);
    let removal_failed = fixture.trace.mark();
    let error = fixture
        .reclaim_without_row_or_event(FINISHED)
        .await
        .expect_err("a failed protection removal fails the reclaim");
    match &error {
        ShimError::MtlsStop(MtlsInterceptStopError::ElementRemoval {
            alloc_id: stopped,
            source,
        }) => {
            assert_eq!(stopped, &alloc_id(FINISHED));
            assert!(
                is_injected_removal_failure(source, address),
                "the removal cause is the injected InterceptError, got {source:?}"
            );
        }
        other => panic!("expected MtlsStop(ElementRemoval), got {other:?}"),
    }
    assert_eq!(
        without_noop_retire(fixture.trace.since(&removal_failed), FINISHED),
        vec![
            Step::DriverStop { alloc: finished(), outcome: StopOutcome::NotFound },
            Step::ElementRemoval {
                source: address,
                destinations: destinations(address),
                outcome: RemovalOutcome::Refused,
            },
        ],
        "no teardown and no lease_released after a failed removal"
    );

    // 2. Teardown fails: its typed cause is returned and the lease is kept.
    fixture.intercept.script_removal_failure(false);
    fixture.sim_owner.script_teardown_failure(true);
    let teardown_failed = fixture.trace.mark();
    let error = fixture
        .reclaim_without_row_or_event(FINISHED)
        .await
        .expect_err("a failed teardown fails the reclaim");
    assert!(
        matches!(
            error,
            ShimError::GuestNetwork(GuestNetworkError::Io {
                operation: GuestNetworkOperation::TapDelete,
                ..
            })
        ),
        "the owner's typed teardown cause is the reclaim error, got {error:?}"
    );
    assert_eq!(
        without_noop_retire(fixture.trace.since(&teardown_failed), FINISHED),
        vec![
            Step::DriverStop { alloc: finished(), outcome: StopOutcome::NotFound },
            Step::ElementRemoval {
                source: address,
                destinations: destinations(address),
                outcome: RemovalOutcome::Removed,
            },
            Step::Owner(GuestNetworkOperation::TapDelete),
        ],
        "the retried removal converges; the refused teardown leaves the lease held"
    );

    // 3. The next attempt completes the teardown and releases the lease last.
    fixture.sim_owner.script_teardown_failure(false);
    let completed = fixture.trace.mark();
    fixture.reclaim_without_row_or_event(FINISHED).await.expect("the next reclaim completes");
    assert_eq!(
        without_noop_retire(fixture.trace.since(&completed), FINISHED),
        vec![
            Step::DriverStop { alloc: finished(), outcome: StopOutcome::NotFound },
            Step::Owner(GuestNetworkOperation::TapDelete),
            Step::LeaseReleased { alloc: finished() },
        ],
        "the protection is already gone, so teardown → lease_released"
    );
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH
/// S-ND295-56 — Reclaim cleans an allocation's network without touching its row
/// CONTRACT_SHAPE: bounded-change.
///
/// E9: a reclaim whose parts were removed out of band releases the lease. The
/// finished allocation's attachment parts leave the owner double's model and
/// its protection members leave the intercept double's model, each out of
/// band. The reclaim's element removal and teardown converge on that absence,
/// and the lease is released last.
#[tokio::test]
#[ignore = "pending DELIVER step 07-02 (S-ND295-56)"]
async fn a_reclaim_whose_parts_were_removed_out_of_band_releases_the_lease() {
    const FINISHED: &str = "nd295-reclaim-out-of-band";
    let fixture = SeamFixture::with_activation_fault_owner().await;
    let address = fixture.finished_allocation_with_a_retiring_lease(FINISHED).await;

    // Out of band: another actor removes the finished allocation's attachment
    // parts and its protection members.
    assert!(
        fixture.owner.remove_parts_out_of_band(&alloc_id(FINISHED)),
        "precondition: the finished allocation's attachment parts were present"
    );
    let members_before = fixture.intercept.members();
    assert!(
        members_before.managed_guest_ips.contains(&address)
            && members_before.outbound_sources.contains(&address)
            && destinations(address)
                .iter()
                .all(|destination| members_before.inbound_destinations.contains(destination)),
        "precondition: the failed removal left the allocation's members in place: \
         {members_before:?}"
    );
    fixture.intercept.remove_members_out_of_band(address, &destinations(address));
    let members_after = fixture.intercept.members();
    assert!(
        !members_after.managed_guest_ips.contains(&address)
            && !members_after.outbound_sources.contains(&address)
            && destinations(address)
                .iter()
                .all(|destination| !members_after.inbound_destinations.contains(destination)),
        "the out-of-band delete removed the allocation's members: {members_after:?}"
    );

    let reclaim = fixture.trace.mark();
    fixture
        .reclaim_without_row_or_event(FINISHED)
        .await
        .expect("a reclaim whose parts were removed out of band completes");

    assert_eq!(
        without_noop_retire(fixture.trace.since(&reclaim), FINISHED),
        vec![
            Step::DriverStop { alloc: FINISHED.to_owned(), outcome: StopOutcome::NotFound },
            Step::ElementRemoval {
                source: address,
                destinations: destinations(address),
                outcome: RemovalOutcome::Removed,
            },
            Step::Owner(GuestNetworkOperation::TapDelete),
            Step::LeaseReleased { alloc: FINISHED.to_owned() },
        ],
        "the removal and teardown converge on the absent parts, then lease_released"
    );
    assert_eq!(
        fixture.owner.teardowns(),
        vec![(alloc_id(FINISHED), false)],
        "the one teardown found the parts already gone"
    );
    assert_eq!(fixture.row_state(FINISHED).await, Some(AllocState::Failed));
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH
/// S-ND295-56 — Reclaim cleans an allocation's network without touching its row
/// CONTRACT_SHAPE: bounded-change.
///
/// The reclaim retires an Admitted lease. A started allocation crashes: its
/// VMM exits through the driver port, and the production exit observer writes
/// its Failed row. Nothing retires the lease, so it is still Admitted. The
/// reclaim records `lease_retired` before the element removal and the owner's
/// teardown, and `lease_released` after them.
#[tokio::test]
#[ignore = "pending DELIVER step 07-02 (S-ND295-56)"]
async fn a_reclaim_retires_an_admitted_lease_before_its_teardown_and_releases_it_after() {
    const CRASHED: &str = "nd295-reclaim-crashed";
    let fixture = SeamFixture::with_sim_owner().await;
    fixture.start_protection_and_open_exec().await;
    let observer_shutdown = tokio_util::sync::CancellationToken::new();
    let observer = overdrive_control_plane::worker::exit_observer::spawn_with_runtime(
        Arc::clone(&fixture.state.obs),
        Arc::clone(&fixture.driver) as Arc<dyn Driver>,
        Arc::clone(&fixture.state.lifecycle_events),
        Arc::clone(&fixture.state.clock),
        None,
        observer_shutdown.clone(),
    );

    let start = fixture.trace.mark();
    fixture.dispatch(start_action(CRASHED)).await.expect("the allocation starts");
    let address = fixture.started_assignment(0).address;
    assert_eq!(fixture.row_state(CRASHED).await, Some(AllocState::Running));

    // The VMM crashes; the production exit observer writes the Failed row.
    let mut lifecycle = fixture.state.lifecycle_events.subscribe();
    fixture.driver.inner.inject_exit_after(
        &alloc_id(CRASHED),
        Duration::ZERO,
        ExitKind::Crashed { exit_code: Some(1), signal: None },
    );
    let crash = tokio::time::timeout(Duration::from_secs(5), lifecycle.recv())
        .await
        .expect("the exit observer records the crash within five seconds")
        .expect("the lifecycle bus stays open");
    assert_eq!(
        crash.alloc_id,
        alloc_id(CRASHED),
        "the observed transition is the crash: {crash:?}"
    );
    assert_eq!(
        fixture.row_state(CRASHED).await,
        Some(AllocState::Failed),
        "precondition: the crash wrote the allocation's Failed row"
    );
    let before_reclaim = fixture.trace.since(&start);
    assert!(
        !before_reclaim.iter().any(|step| matches!(
            step,
            Step::LeaseRetired { alloc } | Step::LeaseReleased { alloc } if alloc == CRASHED
        )),
        "precondition: the start and the crash leave the lease Admitted: {before_reclaim:#?}"
    );

    let reclaim = fixture.trace.mark();
    fixture
        .reclaim_without_row_or_event(CRASHED)
        .await
        .expect("the reclaim of a crashed, Admitted allocation completes");
    let steps = fixture.trace.since(&reclaim);
    let stop = steps
        .iter()
        .find_map(|step| match step {
            Step::DriverStop { alloc, outcome } if alloc == CRASHED => Some(*outcome),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the reclaim confirms the VMM gone: {steps:#?}"));
    assert_ne!(stop, StopOutcome::Failed, "the VMM stop succeeds or finds it gone: {steps:#?}");
    assert_eq!(
        steps,
        vec![
            Step::LeaseRetired { alloc: CRASHED.to_owned() },
            Step::DriverStop { alloc: CRASHED.to_owned(), outcome: stop },
            Step::ElementRemoval {
                source: address,
                destinations: destinations(address),
                outcome: RemovalOutcome::Removed,
            },
            Step::Owner(GuestNetworkOperation::TapDelete),
            Step::LeaseReleased { alloc: CRASHED.to_owned() },
        ],
        "lease_retired precedes the removal and the teardown; lease_released follows them"
    );
    assert_eq!(fixture.row_state(CRASHED).await, Some(AllocState::Failed));

    observer_shutdown.cancel();
    observer.await.expect("the exit observer stops cooperatively");
}
