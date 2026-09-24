//! netns-density-295 correctness-recovery proof §3.4 — fallible element cleanup
//! through the real allocation-stop owner path.
//!
//! # Contract under proof (accepted; `feature-delta.md` C-295 set-element
//! contract + D-295-DISTILL-7, `MtlsInterceptLifecycle::stop_alloc` rustdoc)
//!
//! Normal allocation stop removes the allocation's `2 + P` node-shared set
//! elements (managed guest IP, outbound source, one inbound destination per
//! declared Service port) and reads them back **on the awaited normal path**.
//! "The final normal-path owner calls the one grouped delete/read-back before
//! disarming and dropping its tokens; failure keeps the tokens and allocation
//! retirement ownership retryable. `Drop` is only the non-panicking best-effort
//! unwind/crash fallback … and never converts it into normal-path success."
//! "`complete` alone removes the Retiring record and reservations. A successor
//! using the same address is refused until completion, and the external
//! network owner still releases the address last." The action shim's
//! `stop_alloc` contract: on `Err` the failure "remains a retryable, typed
//! `MtlsInterceptStopError` and retains its owner for a later stop attempt";
//! structural teardown may proceed only on `Ok`; the durable terminal row is
//! written only after every owned effect has converged.
//!
//! # Hypothesis / prediction / falsification (debugging.md §4)
//!
//! - **Hypothesis:** the current stop path hides element cleanup inside the
//!   infallible `InterceptGuard` `Drop` (`drop(drain.take_elements());
//!   drain.complete();` in `MtlsInterceptWorker::begin_stop_alloc`), so a lower
//!   deletion or read-back failure cannot reach its owner.
//! - **Prediction (current code):** `StopAllocation` returns `Ok`; the worker
//!   reports the stop converged and has dropped the capability, so a successor
//!   registration on the same address is admitted; structural guest-network
//!   teardown runs and the guest address is released and re-assigned; the
//!   allocation row is `Terminated`; a later stop never retries the deletion.
//!   The injected fault itself is observed (witness entries GREEN) and the
//!   un-removed elements remain installed.
//! - **Falsification:** `StopAllocation` returns `Err` carrying the injected
//!   cause, the stop is not converged, a same-address successor is refused
//!   with `RegistrationConflict`, no structural teardown/address release
//!   happens, and the retry re-runs the deletion to success before the address
//!   is released.
//!
//! # Fault injection — at an existing driven port, no production seam
//!
//! The fault is injected at the existing [`MtlsIntercept`] port that the real
//! `MtlsInterceptWorker` already consumes. [`ElementFaultIntercept`] delegates
//! the node-program half (bind / converge / observe) to the standing
//! [`SimMtlsIntercept`] and replaces only the per-allocation element half with
//! an in-memory model of the node-shared set membership. Its element removal
//! follows the host adapter's documented failure semantics: a rejected batch
//! preserves the complete pre-state; an acknowledged batch whose read-back
//! fails performs one inverse transition to the captured pre-state. Both
//! return the original cause. The production host adapter
//! (`SharedElementGuard::drop`, `mtls_intercept_port.rs`) behaves the same way
//! at the owner boundary: on a delete failure it restores its process-local
//! refcounts, logs `health.mtls.shared_element_cleanup_failed`, and returns
//! nothing to the worker.
//!
//! The real owners run unmodified: `MtlsInterceptWorker` (capability registry,
//! retirement, drain, stop fence) and the action shim's `StopAllocation` arm
//! (driver stop → mTLS stop → guest-network teardown → address release →
//! terminal row). No kernel state is touched.
//!
//! # Preserving the original failure independently of any cleanup API
//!
//! Every removal goes through one lower effect, [`ElementModel::remove`],
//! which appends each attempt and its exact outcome to an append-only log
//! before returning. Today the only caller is the guard's `Drop`, which has no
//! return channel. A replacement design that adds an awaited, fallible cleanup
//! call on the port routes that call to the same `remove`; the recorded
//! original failure, the membership model, and every assertion below stay
//! unchanged. No assertion names a proposed cleanup API.
//!
//! # Universe (port-exposed observables only)
//!
//! `StopAllocation` dispatch outcome and its `Debug` chain; the lower element
//! effect log and set membership of the injected driven port; the worker's
//! `alloc_stop_converged_for_test`; `MtlsInterceptWorker::start_alloc` result
//! for a same-address successor; the guest-network driven port's recorded
//! `provision` / `teardown` calls and assigned addresses; the allocation's
//! current `AllocStatusRow.state`.
//!
//! # Placement
//!
//! `overdrive-control-plane` owns the `StopAllocation` arm that orders mTLS
//! stop before structural teardown and releases the guest address last, and it
//! already composes `overdrive-worker`; the sibling
//! `mtls_install_fail_closed.rs` drives the same composition. The proof is
//! in-process and deterministic (no schedule dependence), but it needs the
//! `integration-tests`-gated test surfaces
//! (`dispatch_with_guest_network_provisioner_for_test`,
//! `alloc_stop_converged_for_test`), a real redb intent store in a tempdir, and
//! real loopback listener binds from `SimMtlsIntercept`, so it lives in the
//! gated integration lane. It is not a `tests/conformance` test: it observes
//! the crate's own dispatch driving port, not the exported server handler.
//!
//! Run via
//! `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane
//! --features integration-tests -E 'test(shared_element_cleanup_failure)'`.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use overdrive_control_plane::action_shim::{
    ShimError, dispatch_with_guest_network_provisioner_for_test,
};
use overdrive_control_plane::guest_network::{
    GuestNetworkOperation, GuestNetworkPlan, GuestNetworkProvisioner,
};
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::driver::{
    AllocationSpec, Driver, DriverPayload, DriverType, GuestNetworkAssignment, Resources, VmPayload,
};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_sim::adapters::SimIdentityRead;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
use overdrive_sim::adapters::mtls_intercept::SimMtlsIntercept;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::mtls_intercept::InterceptPostcondition;
use overdrive_worker::mtls_intercept_port::{InterceptGuard, MtlsIntercept};
use overdrive_worker::mtls_intercept_worker::{MtlsInterceptInstallError, MtlsInterceptWorker};
use parking_lot::Mutex;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Injected driven port: node-shared set membership with a lower cleanup fault.
// ---------------------------------------------------------------------------

/// One member of the three node-shared sets (`managed_guest_ips`,
/// `outbound_sources`, `inbound_destinations`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SetMember {
    ManagedGuest(Ipv4Addr),
    OutboundSource(Ipv4Addr),
    InboundDestination(SocketAddrV4),
}

/// The lower element-cleanup failure armed on the driven port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LowerCleanupFault {
    /// The deletion batch is rejected; the complete pre-state is preserved.
    DeletionRejected,
    /// The deletion batch is acknowledged, the post-commit read-back fails,
    /// and one inverse transition restores the captured pre-state.
    ReadBackFailed,
}

impl LowerCleanupFault {
    /// Distinctive original cause carried by every failed attempt. A single
    /// token (no quoting-sensitive characters) so it survives any `Debug`
    /// rendering of an error chain that retains the original source.
    const fn cause(self) -> &'static str {
        match self {
            Self::DeletionRejected => "injected-element-delete-rejected-ebusy",
            Self::ReadBackFailed => "injected-element-readback-failed-eio",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CleanupOutcome {
    Removed,
    Failed { fault: LowerCleanupFault, cause: &'static str },
}

/// One append-only record of the lower removal effect.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CleanupAttempt {
    sequence: usize,
    guard: u64,
    members: BTreeSet<SetMember>,
    outcome: CleanupOutcome,
}

/// One append-only record of an element acquisition.
#[derive(Debug, Clone, PartialEq, Eq)]
struct InstallRecord {
    guard: u64,
    members: BTreeSet<SetMember>,
    /// Members that were already present (never removed by a prior owner) and
    /// were therefore silently adopted by this acquisition.
    adopted_residue: BTreeSet<SetMember>,
}

#[derive(Default)]
struct ElementModel {
    members: Mutex<BTreeSet<SetMember>>,
    fault: Mutex<Option<LowerCleanupFault>>,
    attempts: Mutex<Vec<CleanupAttempt>>,
    installs: Mutex<Vec<InstallRecord>>,
    next_guard: AtomicU64,
}

impl ElementModel {
    fn install(self: &Arc<Self>, members: BTreeSet<SetMember>) -> ElementGuard {
        let guard = self.next_guard.fetch_add(1, Ordering::SeqCst) + 1;
        let adopted_residue = {
            let mut set = self.members.lock();
            let residue = members.iter().filter(|m| set.contains(m)).copied().collect();
            set.extend(members.iter().copied());
            residue
        };
        self.installs.lock().push(InstallRecord {
            guard,
            members: members.clone(),
            adopted_residue,
        });
        ElementGuard { model: Arc::clone(self), id: guard, members }
    }

    /// The single lower removal effect. Every present or future cleanup caller
    /// routes here, so the original failure is recorded before any caller
    /// decides what to do with it.
    fn remove(&self, guard: u64, members: &BTreeSet<SetMember>) -> Result<(), &'static str> {
        let fault = *self.fault.lock();
        let outcome = apply_removal(&mut self.members.lock(), fault, members);
        let result = match &outcome {
            CleanupOutcome::Removed => Ok(()),
            CleanupOutcome::Failed { cause, .. } => Err(*cause),
        };
        let mut attempts = self.attempts.lock();
        let sequence = attempts.len();
        attempts.push(CleanupAttempt { sequence, guard, members: members.clone(), outcome });
        result
    }

    fn arm(&self, fault: LowerCleanupFault) {
        *self.fault.lock() = Some(fault);
    }

    fn disarm(&self) {
        *self.fault.lock() = None;
    }

    fn members(&self) -> BTreeSet<SetMember> {
        self.members.lock().clone()
    }

    fn attempts(&self) -> Vec<CleanupAttempt> {
        self.attempts.lock().clone()
    }

    fn installs(&self) -> Vec<InstallRecord> {
        self.installs.lock().clone()
    }
}

/// Apply one removal to the modelled set membership under `fault`.
fn apply_removal(
    set: &mut BTreeSet<SetMember>,
    fault: Option<LowerCleanupFault>,
    members: &BTreeSet<SetMember>,
) -> CleanupOutcome {
    match fault {
        None => {
            for member in members {
                set.remove(member);
            }
            CleanupOutcome::Removed
        }
        Some(fault @ LowerCleanupFault::DeletionRejected) => {
            CleanupOutcome::Failed { fault, cause: fault.cause() }
        }
        Some(fault @ LowerCleanupFault::ReadBackFailed) => {
            let pre_state = set.clone();
            for member in members {
                set.remove(member);
            }
            // Post-commit read-back fails: exactly one inverse transition to
            // the captured pre-state.
            *set = pre_state;
            CleanupOutcome::Failed { fault, cause: fault.cause() }
        }
    }
}

/// Element guard returned by the injected port. The port's
/// [`InterceptGuard`] is marker-only: `Drop` is the only place the current
/// owner lets the release happen, and it has no channel to return a failure.
struct ElementGuard {
    model: Arc<ElementModel>,
    id: u64,
    members: BTreeSet<SetMember>,
}

impl InterceptGuard for ElementGuard {}

impl Drop for ElementGuard {
    fn drop(&mut self) {
        if let Err(cause) = self.model.remove(self.id, &self.members) {
            eprintln!(
                "[element-model] guard {} release failed inside Drop with no return channel: {cause}",
                self.id
            );
        }
    }
}

struct ElementFaultIntercept {
    program: SimMtlsIntercept,
    model: Arc<ElementModel>,
}

impl MtlsIntercept for ElementFaultIntercept {
    fn bind_transparent(
        &self,
        addr: SocketAddrV4,
    ) -> overdrive_worker::mtls_intercept::Result<TcpListener> {
        self.program.bind_transparent(addr)
    }

    fn converge_shared(
        &self,
        prior: Option<&InterceptPostcondition>,
        leg_f: SocketAddrV4,
        leg_c: SocketAddrV4,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        self.program.converge_shared(prior, leg_f, leg_c)
    }

    fn observe_shared(
        &self,
    ) -> overdrive_worker::mtls_intercept::Result<Option<InterceptPostcondition>> {
        self.program.observe_shared()
    }

    fn install_outbound(
        &self,
        source_addr: Ipv4Addr,
        _agent_leg_f_port: u16,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        Ok(Box::new(self.model.install(BTreeSet::from([
            SetMember::ManagedGuest(source_addr),
            SetMember::OutboundSource(source_addr),
        ]))))
    }

    fn install_inbound(
        &self,
        virt: SocketAddrV4,
        _agent_leg_c_port: u16,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        Ok(Box::new(self.model.install(BTreeSet::from([SetMember::InboundDestination(virt)]))))
    }
}

// ---------------------------------------------------------------------------
// Guest-network driven port: records every awaited effect and its address.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct NetworkCall {
    operation: GuestNetworkOperation,
    alloc: AllocationId,
    assignment: GuestNetworkAssignment,
}

#[derive(Default)]
struct RecordingGuestNetwork {
    calls: Mutex<Vec<NetworkCall>>,
}

impl RecordingGuestNetwork {
    fn record(&self, operation: GuestNetworkOperation, plan: &GuestNetworkPlan) {
        self.calls.lock().push(NetworkCall {
            operation,
            alloc: plan.alloc().clone(),
            assignment: plan.assignment().clone(),
        });
    }

    fn calls(&self) -> Vec<NetworkCall> {
        self.calls.lock().clone()
    }

    fn provisioned(&self, alloc: &AllocationId) -> Option<GuestNetworkAssignment> {
        self.calls
            .lock()
            .iter()
            .find(|call| call.operation == GuestNetworkOperation::TapCreate && &call.alloc == alloc)
            .map(|call| call.assignment.clone())
    }
}

fn teardowns(calls: &[NetworkCall], alloc: &AllocationId) -> Vec<Ipv4Addr> {
    calls
        .iter()
        .filter(|call| call.operation == GuestNetworkOperation::TapDelete && &call.alloc == alloc)
        .map(|call| call.assignment.address)
        .collect()
}

#[async_trait::async_trait]
impl GuestNetworkProvisioner for RecordingGuestNetwork {
    async fn provision(
        &self,
        plan: &GuestNetworkPlan,
    ) -> overdrive_control_plane::guest_network::Result<()> {
        self.record(GuestNetworkOperation::TapCreate, plan);
        Ok(())
    }

    async fn activate(
        &self,
        plan: &GuestNetworkPlan,
    ) -> overdrive_control_plane::guest_network::Result<()> {
        self.record(GuestNetworkOperation::TapSetUp, plan);
        Ok(())
    }

    async fn teardown(
        &self,
        plan: &GuestNetworkPlan,
    ) -> overdrive_control_plane::guest_network::Result<()> {
        self.record(GuestNetworkOperation::TapDelete, plan);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Production composition (every orthogonal port a sim double).
// ---------------------------------------------------------------------------

const SERVICE_PORTS: [u16; 2] = [8080, 8443];

fn build_worker(intercept: Arc<dyn MtlsIntercept>) -> Arc<MtlsInterceptWorker> {
    let identity: Arc<dyn IdentityRead> = Arc::new(SimIdentityRead::new(BTreeMap::new(), None));
    let enforcement: Arc<dyn MtlsEnforcement> =
        Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()));
    let resolve: Arc<dyn overdrive_core::traits::mtls_resolve::MtlsResolve> =
        Arc::new(overdrive_sim::adapters::SimMtlsResolve::new(
            BTreeMap::new(),
            overdrive_core::traits::mtls_resolve::MtlsResolution::NonMesh,
        ));
    Arc::new(MtlsInterceptWorker::new(enforcement, resolve, Arc::new(SimClock::new()), intercept))
}

fn build_vip_allocator(
    store: Arc<dyn overdrive_core::traits::intent_store::IntentStore>,
) -> Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>> {
    let cidr = ipnet::Ipv4Net::new(Ipv4Addr::new(10, 96, 0, 1), 32).expect("/32 prefix");
    let range = VipRange::new(vec![cidr], BTreeSet::new()).expect("vip range");
    Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(range, store)))
}

fn tick(counter: u64) -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000 + counter)),
        tick: counter,
        deadline: now + Duration::from_secs(120),
    }
}

fn service_ports() -> Vec<NonZeroU16> {
    SERVICE_PORTS.iter().map(|port| NonZeroU16::new(*port).expect("non-zero port")).collect()
}

fn spec(name: &str, network: Option<GuestNetworkAssignment>) -> AllocationSpec {
    AllocationSpec {
        alloc: AllocationId::new(name).expect("allocation id"),
        identity: SpiffeId::new(&format!(
            "spiffe://overdrive.local/workload/svc-element-cleanup/alloc/{name}"
        ))
        .expect("SPIFFE ID"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/bin/true".to_owned(),
            args: Vec::new(),
            kernel: PathBuf::from("/nonexistent/kernel"),
            rootfs: PathBuf::from("/nonexistent/rootfs"),
        }),
        resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        network,
        service_ports: service_ports(),
    }
}

fn start_action(name: &str) -> (AllocationId, Action) {
    // `network: None` — the shim's own provision seam assigns the address.
    let spec = spec(name, None);
    let alloc = spec.alloc.clone();
    let action = Action::StartAllocation {
        alloc_id: alloc.clone(),
        workload_id: WorkloadId::new("svc-element-cleanup").expect("workload id"),
        node_id: NodeId::new("node-001").expect("node id"),
        spec,
        kind: WorkloadKind::Service,
    };
    (alloc, action)
}

fn allocation_members(address: Ipv4Addr) -> BTreeSet<SetMember> {
    let mut members =
        BTreeSet::from([SetMember::ManagedGuest(address), SetMember::OutboundSource(address)]);
    members.extend(
        SERVICE_PORTS
            .iter()
            .map(|port| SetMember::InboundDestination(SocketAddrV4::new(address, *port))),
    );
    members
}

struct Harness {
    _tmp: TempDir,
    state: Arc<overdrive_control_plane::AppState>,
    worker: Arc<MtlsInterceptWorker>,
    model: Arc<ElementModel>,
    network: Arc<RecordingGuestNetwork>,
    ticks: AtomicU64,
}

impl Harness {
    async fn boot() -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
        let obs =
            Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
        let model = Arc::new(ElementModel::default());
        let worker = build_worker(Arc::new(ElementFaultIntercept {
            program: SimMtlsIntercept::new(),
            model: Arc::clone(&model),
        }));
        worker.start_shared_owner().await.expect("boot-composed shared listener owner is healthy");
        let mut runtime =
            overdrive_control_plane::reconciler_runtime::ReconcilerRuntime::new_with_redb_view_store_for_test(
                tmp.path(),
            )
            .expect("reconciler runtime");
        runtime
            .register(overdrive_control_plane::noop_heartbeat())
            .await
            .expect("register heartbeat");
        let mut state = overdrive_control_plane::AppState::new(
            Arc::clone(&store),
            store_path,
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::new(runtime),
            Arc::new(SimDriver::new(DriverType::Vm)) as Arc<dyn Driver>,
            Arc::new(SimClock::new()),
            Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
            Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
                overdrive_sim::adapters::entropy::SimEntropy::new(34),
            ))),
            Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
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
            worker,
            model,
            network: Arc::new(RecordingGuestNetwork::default()),
            ticks: AtomicU64::new(0),
        }
    }

    async fn dispatch(&self, action: Action) -> Result<(), ShimError> {
        let tick = tick(self.ticks.fetch_add(1, Ordering::SeqCst));
        Box::pin(dispatch_with_guest_network_provisioner_for_test(
            vec![action],
            self.state.as_ref(),
            &tick,
            self.network.as_ref(),
        ))
        .await
    }

    async fn row_state(&self, alloc: &AllocationId) -> Option<AllocState> {
        self.state
            .obs
            .alloc_status_row(alloc)
            .await
            .expect("allocation row read succeeds")
            .map(|row| row.state)
    }
}

// ---------------------------------------------------------------------------
// Fail-closed universe report: every declared entry is evaluated, printed, and
// then asserted together so one run yields a verdict for every entry.
// ---------------------------------------------------------------------------

struct Verdict {
    entry: &'static str,
    expected: String,
    observed: String,
    holds: bool,
}

#[derive(Default)]
struct Report {
    verdicts: Vec<Verdict>,
}

impl Report {
    fn check(&mut self, entry: &'static str, expected: String, observed: String, holds: bool) {
        self.verdicts.push(Verdict { entry, expected, observed, holds });
    }

    fn render(&self) -> String {
        use std::fmt::Write as _;
        self.verdicts.iter().fold(String::new(), |mut out, verdict| {
            writeln!(
                out,
                "  [{}] {}\n        expected: {}\n        observed: {}",
                if verdict.holds { "GREEN" } else { "RED" },
                verdict.entry,
                verdict.expected,
                verdict.observed,
            )
            .expect("writing to a String cannot fail");
            out
        })
    }

    fn violations(&self) -> Vec<&'static str> {
        self.verdicts.iter().filter(|v| !v.holds).map(|v| v.entry).collect()
    }
}

fn union_of(attempts: &[&CleanupAttempt]) -> BTreeSet<SetMember> {
    attempts.iter().flat_map(|attempt| attempt.members.iter().copied()).collect()
}

/// Drive one allocation through a real start, inject `fault` on the lower
/// element cleanup, stop it through the real `StopAllocation` owner path,
/// probe capability and address retention, then retry with the fault cleared.
#[allow(clippy::too_many_lines, reason = "one fail-closed universe per proof case")]
async fn prove_fallible_element_cleanup(fault: LowerCleanupFault) {
    eprintln!("EXECUTED shared_element_cleanup_failure fault={fault:?}");
    let harness = Harness::boot().await;

    // GIVEN a running Service allocation whose 2 + P interception elements
    // are installed through the real start path.
    let (alloc, start) = start_action("element-cleanup-predecessor");
    harness.dispatch(start).await.expect("predecessor reaches Running with interception live");
    let assignment =
        harness.network.provisioned(&alloc).expect("predecessor receives a guest address");
    let address = assignment.address;
    let own_members = allocation_members(address);
    assert_eq!(
        harness.model.members(),
        own_members,
        "setup: exactly the predecessor's 2 + P elements are installed"
    );
    assert_eq!(harness.row_state(&alloc).await, Some(AllocState::Running), "setup: Running");
    let own_guards: BTreeSet<u64> = harness.model.installs().iter().map(|i| i.guard).collect();

    // WHEN the lower element deletion (or its read-back) fails during the
    // allocation's normal stop.
    harness.model.arm(fault);
    let attempts_mark = harness.model.attempts().len();
    let calls_mark = harness.network.calls().len();
    let stop =
        harness.dispatch(Action::StopAllocation { alloc_id: alloc.clone(), terminal: None }).await;

    let attempts = harness.model.attempts();
    let stop_attempts: Vec<&CleanupAttempt> = attempts[attempts_mark..].iter().collect();
    let members_after_stop = harness.model.members();
    let converged_after_stop = harness.worker.alloc_stop_converged_for_test(&alloc);
    let calls = harness.network.calls();
    let stop_teardowns = teardowns(&calls[calls_mark..], &alloc);
    let row_after_stop = harness.row_state(&alloc).await;

    // Same-address successor registration on the worker's real start path.
    let installs_mark = harness.model.installs().len();
    let successor = spec("element-cleanup-same-address", Some(assignment.clone()));
    let same_address = harness.worker.start_alloc(&successor).await;
    let successor_installs = harness.model.installs()[installs_mark..].to_vec();

    // A newly admitted allocation through the real action path.
    let (fresh, fresh_start) = start_action("element-cleanup-fresh-successor");
    let fresh_dispatch = harness.dispatch(fresh_start).await;
    let fresh_address = harness.network.provisioned(&fresh).map(|a| a.address);

    // THEN (retry) with the lower fault cleared, the reconciler re-drives the
    // same stop.
    harness.model.disarm();
    let retry_attempts_mark = harness.model.attempts().len();
    let retry_calls_mark = harness.network.calls().len();
    let retry =
        harness.dispatch(Action::StopAllocation { alloc_id: alloc.clone(), terminal: None }).await;
    let attempts = harness.model.attempts();
    let retry_attempts: Vec<&CleanupAttempt> = attempts[retry_attempts_mark..]
        .iter()
        .filter(|attempt| own_guards.contains(&attempt.guard))
        .collect();
    let members_after_retry = harness.model.members();
    let converged_after_retry = harness.worker.alloc_stop_converged_for_test(&alloc);
    let calls = harness.network.calls();
    let retry_teardowns = teardowns(&calls[retry_calls_mark..], &alloc);
    let row_after_retry = harness.row_state(&alloc).await;

    // ---- evaluate the complete declared universe ----
    let mut report = Report::default();

    // Witnesses: the fault really reached the lower effect and the injected
    // port preserved the pre-state. These must hold in every world; if they
    // do not, the proof is vacuous.
    report.check(
        "witness: the stop attempted removal of every predecessor element and every attempt failed with the injected cause",
        format!("attempted {own_members:?}; all Failed({fault:?})"),
        format!("{stop_attempts:?}"),
        !stop_attempts.is_empty()
            && union_of(&stop_attempts) == own_members
            && stop_attempts.iter().all(|attempt| {
                matches!(attempt.outcome, CleanupOutcome::Failed { fault: f, .. } if f == fault)
            }),
    );
    report.check(
        "witness: the failed cleanup left the predecessor's 2 + P elements installed (pre-state preserved)",
        format!("{own_members:?}"),
        format!("{members_after_stop:?}"),
        members_after_stop == own_members,
    );

    // Contract: the failure surfaces to the stop owner.
    report.check(
        "the allocation stop reports failure instead of success",
        "Err(..) naming the allocation".to_owned(),
        format!("{stop:?}"),
        matches!(&stop, Err(error) if format!("{error:?}").contains(alloc.as_str())),
    );
    report.check(
        "the surfaced failure retains the original lower cause (not replaced or fabricated)",
        format!("Err chain containing {:?}", fault.cause()),
        format!("{stop:?}"),
        matches!(&stop, Err(error) if format!("{error:?}").contains(fault.cause())),
    );

    // Contract: retirement ownership (the capability) is retained.
    report.check(
        "the allocation stop is not reported converged while its elements remain",
        "alloc_stop_converged = false".to_owned(),
        format!("alloc_stop_converged = {converged_after_stop}"),
        !converged_after_stop,
    );
    report.check(
        "a successor registration on the same guest address is refused until cleanup completes",
        format!("Err(RegistrationConflict {{ address: {address} }}) with no element acquisition"),
        format!("{same_address:?}; acquisitions {successor_installs:?}"),
        matches!(
            &same_address,
            Err(MtlsInterceptInstallError::RegistrationConflict { address: refused })
                if *refused == address
        ) && successor_installs.is_empty(),
    );

    // Contract: structural teardown, address release, and the terminal row
    // wait for successful cleanup.
    report.check(
        "no structural guest-network teardown follows the failed element cleanup",
        "no TapDelete for the predecessor".to_owned(),
        format!("TapDelete addresses {stop_teardowns:?}"),
        stop_teardowns.is_empty(),
    );
    report.check(
        "the predecessor's guest address is not re-assigned while its elements remain",
        format!("fresh successor address != {address}"),
        format!("fresh successor address {fresh_address:?} (dispatch {fresh_dispatch:?})"),
        fresh_address.is_some_and(|fresh| fresh != address),
    );
    report.check(
        "the durable allocation row is not written terminal before cleanup converges",
        "Some(Running)".to_owned(),
        format!("{row_after_stop:?}"),
        row_after_stop == Some(AllocState::Running),
    );

    // Contract: the retained ownership is retryable and a retry completes.
    report.check(
        "the retried stop succeeds",
        "Ok(())".to_owned(),
        format!("{retry:?}"),
        retry.is_ok(),
    );
    report.check(
        "the retried stop re-runs the removal of every predecessor element and it succeeds",
        format!("Removed {own_members:?} by the predecessor's own element tokens"),
        format!("{retry_attempts:?}"),
        union_of(&retry_attempts) == own_members
            && retry_attempts.iter().all(|attempt| attempt.outcome == CleanupOutcome::Removed),
    );
    report.check(
        "after the retry none of the predecessor's elements remain installed",
        format!("no member of {own_members:?}"),
        format!("{members_after_retry:?}"),
        members_after_retry.is_disjoint(&own_members),
    );
    report.check(
        "after the retry the allocation stop is converged (ordering is pinned by the not-converged-after-failure entry)",
        "alloc_stop_converged = true".to_owned(),
        format!("alloc_stop_converged = {converged_after_retry}"),
        converged_after_retry,
    );
    report.check(
        "structural teardown and address release happen once, after the successful retry",
        format!("exactly one TapDelete at {address} during the retry"),
        format!("TapDelete addresses during retry {retry_teardowns:?}"),
        retry_teardowns == vec![address],
    );
    report.check(
        "after the retry the allocation row is terminal (ordering is pinned by the non-terminal-before-cleanup entry)",
        "Some(Terminated)".to_owned(),
        format!("{row_after_retry:?}"),
        row_after_retry == Some(AllocState::Terminated),
    );

    // Append-only diagnostic history for this run.
    eprintln!("---- lower element effect log (append-only) ----");
    for attempt in harness.model.attempts() {
        eprintln!("  {attempt:?}");
    }
    eprintln!("---- element acquisitions (append-only) ----");
    for install in harness.model.installs() {
        eprintln!("  {install:?}");
    }
    eprintln!("---- guest-network driven-port calls (append-only) ----");
    for call in harness.network.calls() {
        eprintln!("  {:?} {} {}", call.operation, call.alloc, call.assignment.address);
    }
    eprintln!("---- universe verdicts ({fault:?}) ----\n{}", report.render());

    let violations = report.violations();
    assert!(
        violations.is_empty(),
        "fallible element cleanup contract violated for {fault:?}: {violations:#?}\n{}",
        report.render()
    );
}

/// §3.4 — a rejected lower element deletion during normal allocation stop.
/// Outcome anchor: OUT-ND295-BORN-CAPTURED (cleanup half).
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test]
async fn shared_element_cleanup_failure_deletion_rejected_retains_retirement_and_address() {
    Box::pin(prove_fallible_element_cleanup(LowerCleanupFault::DeletionRejected)).await;
}

/// §3.4 — an acknowledged deletion whose post-commit read-back fails (and is
/// restored to the pre-state) during normal allocation stop.
/// Outcome anchor: OUT-ND295-BORN-CAPTURED (cleanup half).
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test]
async fn shared_element_cleanup_failure_readback_failed_retains_retirement_and_address() {
    Box::pin(prove_fallible_element_cleanup(LowerCleanupFault::ReadBackFailed)).await;
}
