//! GH #295 — the node-shared mTLS listener owner, driven through the accepted
//! `MtlsInterceptWorker` surface over a recording `MtlsIntercept` double.
//!
//! **Lane: `integration-tests`.** `RecordingSharedIntercept::bind_transparent`
//! binds real loopback listening sockets, and the bodies make real client
//! connections from allocation source addresses and prove real socket release
//! by rebinding the exact addresses. Real sockets are real infrastructure under
//! `.claude/rules/testing.md` § "Integration vs unit gating", so this file lives
//! in the gated integration binary, not the default-lane acceptance binary.
//! After the B-7 step the double returns a `LoopbackInterceptListener`, which
//! still binds a real loopback socket, so the lane does not change then.
//!
//! Faults enter only through the recording intercept (the driven port): a
//! refused bind or convergence, a lost listener, a lost member, a lost policy
//! route or intercept-mark guard, and an absent or differently targeted
//! program. The oracles read the worker's typed results and the port's
//! recorded state; no fixture writes the outcome an oracle observes.

#![allow(clippy::doc_markdown)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener};
use std::os::fd::{AsRawFd as _, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex as StdMutex};
use std::time::Duration;

use overdrive_core::guest_network::SharedGuestNetworkComponent;
use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::driver::{
    AllocationSpec, DriverPayload, GuestNetworkAssignment, Resources, VmPayload,
};
use overdrive_core::traits::mtls_enforcement::{
    EnforcedConnection, EnforcedConnectionId, InterceptedConnection, MtlsEnforcement, MtlsLimits,
    PumpLiveness,
};
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve, ResolvedBackend};
use overdrive_netlink::nft::SharedIpInterceptIdentity;
use overdrive_sim::adapters::SimIdentityRead;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
use overdrive_worker::mtls_intercept::{InterceptError, InterceptLeg, InterceptPostcondition};
use overdrive_worker::mtls_intercept_port::{
    InterceptAcceptError, InterceptAccepted, InterceptGuard, InterceptListener, InterceptMembers,
    InterceptState, MtlsIntercept,
};
use overdrive_worker::mtls_intercept_worker::{MtlsInterceptWorker, MtlsSharedOwnerError};
use parking_lot::Mutex;

fn worker(intercept: Arc<dyn MtlsIntercept>) -> Arc<MtlsInterceptWorker> {
    let identity: Arc<dyn IdentityRead> = Arc::new(SimIdentityRead::new(BTreeMap::new(), None));
    let enforcement: Arc<dyn MtlsEnforcement> =
        Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()));
    let resolve: Arc<dyn MtlsResolve> = Arc::new(overdrive_sim::adapters::SimMtlsResolve::new(
        BTreeMap::new(),
        MtlsResolution::NonMesh,
    ));
    Arc::new(MtlsInterceptWorker::new(enforcement, resolve, Arc::new(SimClock::new()), intercept))
}

fn worker_with_enforcement(
    intercept: Arc<dyn MtlsIntercept>,
    enforcement: Arc<dyn MtlsEnforcement>,
) -> Arc<MtlsInterceptWorker> {
    let peer = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 9443);
    let resolve: Arc<dyn MtlsResolve> = Arc::new(overdrive_sim::adapters::SimMtlsResolve::new(
        BTreeMap::new(),
        MtlsResolution::Mesh(ResolvedBackend { addr: peer, expected_svid: None }),
    ));
    Arc::new(MtlsInterceptWorker::new(enforcement, resolve, Arc::new(SimClock::new()), intercept))
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn shared_owner_starts_once_audits_and_shutdown_drains_the_owner_tree() {
    let intercept = Arc::new(RecordingSharedIntercept::new());
    let worker = worker(intercept.clone());
    let before = intercept.surface();

    worker.start_shared_owner().await.expect("two listeners, two tasks, and one node guard start");
    worker.audit_shared_owner().await.expect("full listener/task/rule read-back succeeds");
    let published = intercept.surface();
    assert_eq!(before.call_counts, (0, 0, 0));
    assert_eq!(published.call_counts, (2, 1, 4));
    assert_eq!(published.listener_addresses.len(), 2);
    assert_ne!(published.listener_addresses[0], published.listener_addresses[1]);
    assert!(published.listener_addresses.iter().all(|address| address.port() != 0));
    assert_eq!(published.listener_clones, 2, "one live socket exists for each recorded leg");
    assert_eq!(published.shared_guard_drops, 0, "the published node guard remains retained");
    assert_eq!(published.allocation_guard_drops, 0);
    let expected = RecordingSharedIntercept::identity(
        published.listener_addresses[0],
        published.listener_addresses[1],
    );
    assert_eq!(published.observation, Some(expected));

    worker
        .start_shared_owner()
        .await
        .expect("repeated owner start is idempotent and adds no listener or task");
    assert_eq!(
        intercept.surface(),
        published,
        "idempotent start changes no socket, task, guard, program, or call cardinality"
    );

    intercept.release_listener_clones();
    worker.shutdown_owner().await.expect("owner shutdown drains the complete userspace tree");
    let shutdown = intercept.surface();
    assert_eq!(shutdown.call_counts, published.call_counts);
    assert_eq!(shutdown.listener_addresses, published.listener_addresses);
    assert_eq!(shutdown.observation, published.observation);
    assert_eq!(shutdown.listener_clones, 0);
    assert_eq!(shutdown.shared_guard_drops, 0, "sealed shutdown relinquishes the node guard");
    assert_eq!(shutdown.allocation_guard_drops, 0);
    for address in published.listener_addresses {
        drop(
            TcpListener::bind(address).expect("shutdown closes each exact shared listener socket"),
        );
    }
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn initial_leg_f_bind_refusal_returns_to_absent_without_partial_publication() {
    for errno in [libc::EADDRINUSE, libc::EPERM, libc::EMFILE] {
        let intercept = Arc::new(RecordingSharedIntercept::failing_bind_with_errno(1, errno));
        let worker = worker(intercept.clone());
        let before = intercept.surface();

        let error =
            worker.start_shared_owner().await.expect_err("standing bind fault refuses start");
        assert!(matches!(error, MtlsSharedOwnerError::ListenerBind { leg: InterceptLeg::F, .. }));
        assert!(matches!(worker.audit_shared_owner().await, Err(MtlsSharedOwnerError::NotStarted)));
        worker.shutdown_owner().await.expect("Absent owner shutdown is idempotent");
        let after = intercept.surface();
        let mut expected = before;
        expected.call_counts = (1, 0, 1);
        assert_eq!(after, expected, "leg-F refusal changes only observe/bind call receipts");
    }
}

/// Recording `MtlsIntercept` double modelling the host adapter's observable
/// state: the owned program, the three member sets, the policy route, the
/// intercept-mark guard table, and the adapter's recorded identity (the
/// host's `targets`/`program`, set by `converge_shared` and cleared by the
/// node guard's `Drop`).
///
/// The S-ND295-13D / S-ND295-61 support is the call journal, the one-shot
/// `converge_allocation_elements` outcome slot, and the kernel-side loss knobs
/// (`lose_member`, `lose_policy_route`, `lose_intercept_mark_guard`,
/// `delete_program`, `replace_shared_observation`). Every knob changes only the
/// port's state; the worker's reaction is what the bodies observe.
struct RecordingSharedIntercept {
    listener_clones: Mutex<Vec<Arc<LoopbackInterceptListener>>>,
    listener_addresses: Mutex<Vec<SocketAddrV4>>,
    retain_listener_clones: bool,
    bind_calls: AtomicUsize,
    converge_calls: AtomicUsize,
    /// Worker observation calls: `observe_shared` and `observe_shared_state`.
    /// The adapter's own read-backs inside member effects are not counted.
    observe_calls: AtomicUsize,
    fail_bind_at: Option<usize>,
    fail_bind_errno: Option<i32>,
    fail_converge: AtomicBool,
    /// The owned program as the kernel holds it; `None` means the owned table
    /// is absent.
    shared_observation: Arc<Mutex<Option<InterceptPostcondition>>>,
    shared_guard_drops: Arc<AtomicUsize>,
    allocation_guard_drops: Arc<AtomicUsize>,
    /// The members the element guards own; the kernel holds these minus
    /// `lost_members`.
    allocation_elements: Arc<Mutex<BTreeSet<SharedElement>>>,
    retired_elements: Arc<Mutex<BTreeSet<SharedElement>>>,
    occupy_exact_rebind: AtomicBool,
    blockers: Mutex<Vec<TcpListener>>,
    /// Every port call, in order.
    journal: Mutex<Vec<InterceptCall>>,
    /// The identity the last successful `converge_shared` recorded; cleared by
    /// the node guard's `Drop`, as the host adapter's guard clears its targets.
    recorded_identity: Arc<Mutex<Option<InterceptPostcondition>>>,
    /// Members deleted from the kernel behind the worker's back.
    lost_members: Arc<Mutex<BTreeSet<SharedElement>>>,
    policy_route: AtomicBool,
    intercept_mark_guard: AtomicBool,
    /// Committed program creates and target replacements.
    program_writes: AtomicUsize,
    converge_elements_script: Mutex<Option<ConvergeElementsScript>>,
}

/// One call the worker made on the recording port.
#[derive(Debug, Clone, PartialEq, Eq)]
enum InterceptCall {
    BindTransparent(SocketAddrV4),
    ConvergeShared {
        prior: Option<InterceptPostcondition>,
        leg_f: SocketAddrV4,
        leg_c: SocketAddrV4,
    },
    ObserveShared,
    InstallOutbound(Ipv4Addr),
    InstallInbound(SocketAddrV4),
    ObserveSharedState,
    ConvergeAllocationElements(InterceptMembers),
    RemoveAllocationElements {
        source_addr: Ipv4Addr,
        destinations: Vec<SocketAddrV4>,
    },
}

impl InterceptCall {
    /// A read of the owned program: the two observation methods.
    const fn reads_the_program(&self) -> bool {
        matches!(self, Self::ObserveShared | Self::ObserveSharedState)
    }
}

/// The scripted outcome of the next `converge_allocation_elements` call.
#[derive(Debug, Clone, Copy)]
enum ConvergeElementsScript {
    /// The batch is rejected with this errno and nothing changes.
    Refuse { errno: i32 },
    /// The call reports success but its read-back still shows the members the
    /// kernel held before it: the clear did not take.
    LeaveMembers,
}

/// The typed error a scripted member-convergence refusal returns.
fn member_convergence_refused(errno: i32) -> InterceptError {
    InterceptError::NftRuleInstallFailed {
        op: "converge-shared-members",
        source: overdrive_worker::mtls_intercept::NetlinkError::nft(
            "converge-shared-members",
            std::io::Error::from_raw_os_error(errno),
        ),
    }
}

/// The typed refusal of a program create or target replacement while members
/// exist: the host's program write is strict about empty sets (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the runtime repair contract: program writes stay strict)).
fn program_write_over_members(
    prior: Option<InterceptPostcondition>,
    requested: InterceptPostcondition,
) -> InterceptError {
    InterceptError::NftSharedReplaceFailed {
        prior,
        requested,
        source: overdrive_worker::mtls_intercept::NetlinkError::nft(
            "replace-shared",
            std::io::Error::from_raw_os_error(libc::ENOTEMPTY),
        ),
    }
}

impl RecordingSharedIntercept {
    fn new() -> Self {
        Self {
            listener_clones: Mutex::new(Vec::new()),
            listener_addresses: Mutex::new(Vec::new()),
            retain_listener_clones: true,
            bind_calls: AtomicUsize::new(0),
            converge_calls: AtomicUsize::new(0),
            observe_calls: AtomicUsize::new(0),
            fail_bind_at: None,
            fail_bind_errno: None,
            fail_converge: AtomicBool::new(false),
            shared_observation: Arc::new(Mutex::new(None)),
            shared_guard_drops: Arc::new(AtomicUsize::new(0)),
            allocation_guard_drops: Arc::new(AtomicUsize::new(0)),
            allocation_elements: Arc::new(Mutex::new(BTreeSet::new())),
            retired_elements: Arc::new(Mutex::new(BTreeSet::new())),
            occupy_exact_rebind: AtomicBool::new(false),
            blockers: Mutex::new(Vec::new()),
            journal: Mutex::new(Vec::new()),
            recorded_identity: Arc::new(Mutex::new(None)),
            lost_members: Arc::new(Mutex::new(BTreeSet::new())),
            policy_route: AtomicBool::new(true),
            intercept_mark_guard: AtomicBool::new(true),
            program_writes: AtomicUsize::new(0),
            converge_elements_script: Mutex::new(None),
        }
    }

    /// The kernel state a killed prior process left: its owned program and
    /// its members, with no identity recorded in this fresh process.
    fn left_by_a_prior_process(
        program: InterceptPostcondition,
        members: &BTreeSet<SharedElement>,
    ) -> Self {
        let intercept = Self::new();
        *intercept.shared_observation.lock() = Some(program);
        intercept.allocation_elements.lock().clone_from(members);
        intercept
    }

    fn record(&self, call: InterceptCall) {
        self.journal.lock().push(call);
    }

    fn journal(&self) -> Vec<InterceptCall> {
        self.journal.lock().clone()
    }

    fn journal_len(&self) -> usize {
        self.journal.lock().len()
    }

    /// The owned state as the kernel holds it, read without recording a call.
    fn kernel_state(&self) -> Option<InterceptState> {
        let program = self.shared_observation.lock().clone()?;
        let held: BTreeSet<SharedElement> = self.allocation_elements.lock().clone();
        let lost: BTreeSet<SharedElement> = self.lost_members.lock().clone();
        let present: BTreeSet<SharedElement> = held.difference(&lost).copied().collect();
        Some(InterceptState {
            program,
            policy_route: self.policy_route.load(Ordering::SeqCst),
            intercept_mark_guard: self.intercept_mark_guard.load(Ordering::SeqCst),
            members: intercept_members(&present),
        })
    }

    fn recorded_identity(&self) -> Option<InterceptPostcondition> {
        self.recorded_identity.lock().clone()
    }

    fn program_writes(&self) -> usize {
        self.program_writes.load(Ordering::SeqCst)
    }

    fn script_converge_elements(&self, script: ConvergeElementsScript) {
        *self.converge_elements_script.lock() = Some(script);
    }

    /// A member-set batch that lands in the kernel on its own schedule: the
    /// kernel members become exactly `members`.
    fn land_member_batch(&self, members: &InterceptMembers) {
        *self.allocation_elements.lock() = member_elements(members);
        self.lost_members.lock().clear();
    }

    /// `nft delete element` of one present member.
    fn lose_member(&self, element: SharedElement) {
        assert!(
            self.allocation_elements.lock().contains(&element),
            "only a present member can be deleted from the kernel"
        );
        assert!(self.lost_members.lock().insert(element), "the member is deleted once");
    }

    /// `ip rule del fwmark 0x1 lookup 100` (or its table-100 local route).
    fn lose_policy_route(&self) {
        self.policy_route.store(false, Ordering::SeqCst);
    }

    /// `nft delete table ip overdrive-mtls-guard` (D-295-R18).
    fn lose_intercept_mark_guard(&self) {
        self.intercept_mark_guard.store(false, Ordering::SeqCst);
    }

    /// `nft delete table ip overdrive-mtls`: the program and its sets go
    /// together, so every member leaves the kernel with it.
    fn delete_program(&self) {
        *self.shared_observation.lock() = None;
        let held: BTreeSet<SharedElement> = self.allocation_elements.lock().clone();
        self.lost_members.lock().extend(held);
    }

    fn failing_bind(call: usize) -> Self {
        Self { retain_listener_clones: false, fail_bind_at: Some(call), ..Self::new() }
    }

    fn failing_bind_with_errno(call: usize, errno: i32) -> Self {
        Self {
            retain_listener_clones: false,
            fail_bind_at: Some(call),
            fail_bind_errno: Some(errno),
            ..Self::new()
        }
    }

    fn failing_converge() -> Self {
        Self { retain_listener_clones: false, fail_converge: AtomicBool::new(true), ..Self::new() }
    }

    fn listener_addresses(&self) -> Vec<SocketAddrV4> {
        self.listener_addresses.lock().clone()
    }

    fn shared_observation(&self) -> Option<InterceptPostcondition> {
        self.shared_observation.lock().clone()
    }

    fn identity(leg_f: SocketAddrV4, leg_c: SocketAddrV4) -> InterceptPostcondition {
        let (table_and_chains, sets, prerouting, output) =
            SharedIpInterceptIdentity::for_listener_ports(leg_f.port(), leg_c.port())
                .expect("non-zero shared targets form one canonical identity")
                .normalized_parts();
        InterceptPostcondition::ConstantRules { table_and_chains, sets, prerouting, output }
    }

    fn surface(&self) -> SharedOwnerSurface {
        SharedOwnerSurface {
            call_counts: self.call_counts(),
            listener_addresses: self.listener_addresses(),
            listener_clones: self.listener_clone_count(),
            observation: self.shared_observation(),
            shared_guard_drops: self.shared_guard_drops(),
            allocation_guard_drops: self.allocation_guard_drops(),
            allocation_elements: self.allocation_elements.lock().clone(),
        }
    }

    fn replace_shared_observation(&self, observation: Option<InterceptPostcondition>) {
        *self.shared_observation.lock() = observation;
    }

    fn call_counts(&self) -> (usize, usize, usize) {
        (
            self.bind_calls.load(Ordering::SeqCst),
            self.converge_calls.load(Ordering::SeqCst),
            self.observe_calls.load(Ordering::SeqCst),
        )
    }

    fn shared_guard_drops(&self) -> usize {
        self.shared_guard_drops.load(Ordering::SeqCst)
    }

    fn allocation_guard_drops(&self) -> usize {
        self.allocation_guard_drops.load(Ordering::SeqCst)
    }

    fn listener_clone_count(&self) -> usize {
        self.listener_clones.lock().len()
    }

    fn release_listener_clones(&self) {
        self.listener_clones.lock().clear();
    }

    fn terminate_listener_task(&self, index: usize) {
        let listener = self.listener_clones.lock().remove(index);
        listener.lose();
        drop(listener);
    }

    fn occupy_next_exact_rebind(&self) {
        self.occupy_exact_rebind.store(true, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SharedOwnerSurface {
    call_counts: (usize, usize, usize),
    listener_addresses: Vec<SocketAddrV4>,
    listener_clones: usize,
    observation: Option<InterceptPostcondition>,
    shared_guard_drops: usize,
    allocation_guard_drops: usize,
    allocation_elements: BTreeSet<SharedElement>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SharedElement {
    ManagedGuest(Ipv4Addr),
    OutboundSource(Ipv4Addr),
    InboundDestination(SocketAddrV4),
}

fn allocation_elements(spec: &AllocationSpec) -> BTreeSet<SharedElement> {
    let address = spec.network.as_ref().expect("shared allocation has a network").address;
    let mut elements = BTreeSet::from([
        SharedElement::ManagedGuest(address),
        SharedElement::OutboundSource(address),
    ]);
    elements.extend(
        spec.service_ports
            .iter()
            .map(|port| SharedElement::InboundDestination(SocketAddrV4::new(address, port.get()))),
    );
    elements
}

struct RecordingElementGuard {
    active: Arc<Mutex<BTreeSet<SharedElement>>>,
    /// Elements `remove_allocation_elements` already removed: their tokens are
    /// retired, so this guard's `Drop` performs no effect for them.
    retired: Arc<Mutex<BTreeSet<SharedElement>>>,
    owned: BTreeSet<SharedElement>,
    drops: Arc<AtomicUsize>,
}

impl InterceptGuard for RecordingElementGuard {}

impl Drop for RecordingElementGuard {
    fn drop(&mut self) {
        let mut retired = self.retired.lock();
        let still_owned: Vec<SharedElement> =
            self.owned.iter().filter(|element| !retired.remove(*element)).copied().collect();
        drop(retired);
        let mut active = self.active.lock();
        for element in &still_owned {
            assert!(active.remove(element), "the exact allocation element remains singly owned");
        }
        drop(active);
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

/// The dynamic members one element set holds.
fn intercept_members(elements: &BTreeSet<SharedElement>) -> InterceptMembers {
    let mut members = InterceptMembers::default();
    for element in elements {
        match *element {
            SharedElement::ManagedGuest(ip) => {
                members.managed_guest_ips.insert(ip);
            }
            SharedElement::OutboundSource(ip) => {
                members.outbound_sources.insert(ip);
            }
            SharedElement::InboundDestination(destination) => {
                members.inbound_destinations.insert(destination);
            }
        }
    }
    members
}

/// The element set one member set names.
fn member_elements(members: &InterceptMembers) -> BTreeSet<SharedElement> {
    members
        .managed_guest_ips
        .iter()
        .map(|ip| SharedElement::ManagedGuest(*ip))
        .chain(members.outbound_sources.iter().map(|ip| SharedElement::OutboundSource(*ip)))
        .chain(
            members
                .inbound_destinations
                .iter()
                .map(|destination| SharedElement::InboundDestination(*destination)),
        )
        .collect()
}

/// The typed refusal for a member effect while no program is recorded (DISTILL
/// gap B-8); this double conflates the record with the observed program.
const fn program_not_published() -> InterceptError {
    InterceptError::SharedProgramNotConverged
}

/// The members one allocation's removal requests:
/// `{managed(source), outbound(source)} ∪ {inbound(d)}`.
fn requested_elements(
    source_addr: Ipv4Addr,
    destinations: &[SocketAddrV4],
) -> BTreeSet<SharedElement> {
    [SharedElement::ManagedGuest(source_addr), SharedElement::OutboundSource(source_addr)]
        .into_iter()
        .chain(
            destinations.iter().map(|destination| SharedElement::InboundDestination(*destination)),
        )
        .collect()
}

/// Convergent removal of one allocation's members over a recorded element set:
/// present requested elements are removed and their guards' tokens retired.
fn remove_elements(
    active: &Mutex<BTreeSet<SharedElement>>,
    retired: &Mutex<BTreeSet<SharedElement>>,
    source_addr: Ipv4Addr,
    destinations: &[SocketAddrV4],
) {
    let requested = requested_elements(source_addr, destinations);
    let mut active = active.lock();
    let mut retired = retired.lock();
    for element in requested {
        if active.remove(&element) {
            retired.insert(element);
        }
    }
}

/// The recording's node guard. Like the host's `SharedInterceptGuard`, its
/// `Drop` deletes the program it armed when that exact program is present and
/// the sets are empty (the strict conditional delete refuses otherwise), and
/// always clears the adapter's recorded identity, after which
/// `install_outbound` refuses.
struct RecordingNodeGuard {
    requested: InterceptPostcondition,
    drops: Arc<AtomicUsize>,
    program: Arc<Mutex<Option<InterceptPostcondition>>>,
    recorded: Arc<Mutex<Option<InterceptPostcondition>>>,
    held: Arc<Mutex<BTreeSet<SharedElement>>>,
    lost: Arc<Mutex<BTreeSet<SharedElement>>>,
}

impl InterceptGuard for RecordingNodeGuard {}

impl Drop for RecordingNodeGuard {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        let held: BTreeSet<SharedElement> = self.held.lock().clone();
        let lost: BTreeSet<SharedElement> = self.lost.lock().clone();
        let sets_empty = held.is_subset(&lost);
        let mut program = self.program.lock();
        if sets_empty && program.as_ref() == Some(&self.requested) {
            *program = None;
        }
        drop(program);
        self.recorded.lock().take();
    }
}

/// The listener type `MtlsIntercept::bind_transparent` returns. The DELIVER
/// step that carries B-7 (05-01 at the latest) changes it to
/// `Arc<dyn InterceptListener>` (FD § "[REF] Driven port — intercept listener
/// (DISTILL gap B-7) — pinned 2026-09-25" (the pinned `bind_transparent`
/// signature)). `ActivationBarrierIntercept` delegates to
/// `RecordingSharedIntercept` and is unchanged by that step; the recording's
/// own bind then returns a `LoopbackInterceptListener` (TS § "When the port
/// changes", line 3).
type BoundListener = Arc<dyn InterceptListener>;

impl MtlsIntercept for RecordingSharedIntercept {
    fn bind_transparent(
        &self,
        addr: SocketAddrV4,
    ) -> overdrive_worker::mtls_intercept::Result<BoundListener> {
        self.record(InterceptCall::BindTransparent(addr));
        let call = self.bind_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail_bind_at == Some(call) {
            return Err(InterceptError::TransparentListener {
                addr,
                source: std::io::Error::from_raw_os_error(
                    self.fail_bind_errno.unwrap_or(libc::EMFILE),
                ),
            });
        }
        if addr.port() != 0 && self.occupy_exact_rebind.swap(false, Ordering::SeqCst) {
            let blocker = TcpListener::bind(addr)
                .map_err(|source| InterceptError::TransparentListener { addr, source })?;
            self.blockers.lock().push(blocker);
        }
        let listener = Arc::new(
            LoopbackInterceptListener::bind(addr)
                .map_err(|source| InterceptError::TransparentListener { addr, source })?,
        );
        let bound = listener
            .local_addr()
            .map_err(|source| InterceptError::TransparentListener { addr, source })?;
        self.listener_addresses.lock().push(bound);
        if self.retain_listener_clones {
            self.listener_clones.lock().push(Arc::clone(&listener));
        }
        Ok(listener as BoundListener)
    }

    fn converge_shared(
        &self,
        prior: Option<&InterceptPostcondition>,
        leg_f: SocketAddrV4,
        leg_c: SocketAddrV4,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        self.record(InterceptCall::ConvergeShared { prior: prior.cloned(), leg_f, leg_c });
        self.converge_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_converge.load(Ordering::SeqCst) {
            return Err(InterceptError::NftRuleInstallFailed {
                op: "replace-shared",
                source: overdrive_worker::mtls_intercept::NetlinkError::nft(
                    "replace-shared",
                    std::io::Error::from_raw_os_error(libc::EBUSY),
                ),
            });
        }
        let (table_and_chains, sets, prerouting, output) =
            SharedIpInterceptIdentity::for_listener_ports(leg_f.port(), leg_c.port())
                .map_err(|source| InterceptError::NftRuleInstallFailed {
                    op: "shared-ip-expected",
                    source,
                })?
                .normalized_parts();
        let requested =
            InterceptPostcondition::ConstantRules { table_and_chains, sets, prerouting, output };
        // The host's pre-check: the observed program must be the caller's prior.
        let observed = self.shared_observation.lock().clone();
        if observed.as_ref() != prior {
            return Err(InterceptError::PostconditionMismatch {
                expected: prior.cloned().unwrap_or_else(|| requested.clone()),
                observed,
            });
        }
        // An equal identity writes no program; a create or a target
        // replacement is strict about empty sets.
        if observed.as_ref() != Some(&requested) {
            let members_present = self
                .kernel_state()
                .is_some_and(|state| state.members != InterceptMembers::default());
            if members_present {
                return Err(program_write_over_members(observed, requested));
            }
            *self.shared_observation.lock() = Some(requested.clone());
            self.program_writes.fetch_add(1, Ordering::SeqCst);
        }
        // The policy route and the intercept-mark guard table are ensured
        // add-if-missing on every successful converge.
        self.policy_route.store(true, Ordering::SeqCst);
        self.intercept_mark_guard.store(true, Ordering::SeqCst);
        *self.recorded_identity.lock() = Some(requested.clone());
        Ok(Box::new(RecordingNodeGuard {
            requested,
            drops: Arc::clone(&self.shared_guard_drops),
            program: Arc::clone(&self.shared_observation),
            recorded: Arc::clone(&self.recorded_identity),
            held: Arc::clone(&self.allocation_elements),
            lost: Arc::clone(&self.lost_members),
        }))
    }

    fn observe_shared(
        &self,
    ) -> overdrive_worker::mtls_intercept::Result<Option<InterceptPostcondition>> {
        self.record(InterceptCall::ObserveShared);
        self.observe_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.shared_observation.lock().clone())
    }

    fn install_outbound(
        &self,
        source_addr: Ipv4Addr,
        _leg_f_port: u16,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        self.record(InterceptCall::InstallOutbound(source_addr));
        // The host admits a source only while a recorded owner exists.
        if self.recorded_identity.lock().is_none() {
            return Err(InterceptError::SharedProgramNotConverged);
        }
        let owned = BTreeSet::from([
            SharedElement::ManagedGuest(source_addr),
            SharedElement::OutboundSource(source_addr),
        ]);
        self.allocation_elements.lock().extend(owned.iter().copied());
        self.lost_members.lock().retain(|element| !owned.contains(element));
        Ok(Box::new(RecordingElementGuard {
            active: Arc::clone(&self.allocation_elements),
            retired: Arc::clone(&self.retired_elements),
            owned,
            drops: Arc::clone(&self.allocation_guard_drops),
        }))
    }

    fn install_inbound(
        &self,
        virt: SocketAddrV4,
        _leg_c_port: u16,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        self.record(InterceptCall::InstallInbound(virt));
        let owned = BTreeSet::from([SharedElement::InboundDestination(virt)]);
        self.allocation_elements.lock().extend(owned.iter().copied());
        self.lost_members.lock().retain(|element| !owned.contains(element));
        Ok(Box::new(RecordingElementGuard {
            active: Arc::clone(&self.allocation_elements),
            retired: Arc::clone(&self.retired_elements),
            owned,
            drops: Arc::clone(&self.allocation_guard_drops),
        }))
    }

    fn observe_shared_state(
        &self,
    ) -> overdrive_worker::mtls_intercept::Result<Option<InterceptState>> {
        self.record(InterceptCall::ObserveSharedState);
        self.observe_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.kernel_state())
    }

    fn converge_allocation_elements(
        &self,
        expected: &InterceptMembers,
    ) -> overdrive_worker::mtls_intercept::Result<Option<InterceptState>> {
        self.record(InterceptCall::ConvergeAllocationElements(expected.clone()));
        let script = self.converge_elements_script.lock().take();
        match script {
            Some(ConvergeElementsScript::Refuse { errno }) => {
                return Err(member_convergence_refused(errno));
            }
            Some(ConvergeElementsScript::LeaveMembers) => return Ok(self.kernel_state()),
            None => {}
        }
        let Some(program) = self.shared_observation.lock().clone() else {
            return Ok(None);
        };
        // With a recorded identity the program must equal it; without one (a
        // fresh process) any canonical program is accepted.
        if let Some(recorded) = self.recorded_identity()
            && program != recorded
        {
            return Err(InterceptError::PostconditionMismatch {
                expected: recorded,
                observed: Some(program),
            });
        }
        self.land_member_batch(expected);
        Ok(self.kernel_state())
    }

    fn remove_allocation_elements(
        &self,
        source_addr: Ipv4Addr,
        destinations: &[SocketAddrV4],
    ) -> overdrive_worker::mtls_intercept::Result<InterceptState> {
        self.record(InterceptCall::RemoveAllocationElements {
            source_addr,
            destinations: destinations.to_vec(),
        });
        let program = self.shared_observation.lock().clone().ok_or_else(program_not_published)?;
        if let Some(recorded) = self.recorded_identity()
            && program != recorded
        {
            return Err(InterceptError::PostconditionMismatch {
                expected: recorded,
                observed: Some(program),
            });
        }
        remove_elements(
            &self.allocation_elements,
            &self.retired_elements,
            source_addr,
            destinations,
        );
        let requested = requested_elements(source_addr, destinations);
        self.lost_members.lock().retain(|element| !requested.contains(element));
        self.kernel_state().ok_or_else(program_not_published)
    }
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn leg_c_bind_refusal_closes_the_already_bound_leg_f_and_publishes_no_owner() {
    let intercept = Arc::new(RecordingSharedIntercept::failing_bind(2));
    let worker = worker(intercept.clone());
    let before = intercept.surface();
    let error = worker.start_shared_owner().await.expect_err("second bind refuses start");
    assert!(matches!(error, MtlsSharedOwnerError::ListenerBind { leg: InterceptLeg::C, .. }));
    let addresses = intercept.listener_addresses();
    assert_eq!(addresses.len(), 1, "only leg F was acquired before refusal");
    let rebound = TcpListener::bind(addresses[0]).expect("failed start closes the partial leg F");
    drop(rebound);
    assert!(matches!(worker.audit_shared_owner().await, Err(MtlsSharedOwnerError::NotStarted)));
    worker.shutdown_owner().await.expect("Absent owner shutdown is idempotent");
    let after = intercept.surface();
    let mut expected = before;
    expected.call_counts = (2, 0, 1);
    expected.listener_addresses = addresses;
    assert_eq!(after, expected, "leg-C refusal changes only the exact partial-bind receipts");
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn shared_rule_convergence_refusal_closes_both_sockets_and_publishes_no_tasks_or_guard() {
    let intercept = Arc::new(RecordingSharedIntercept::failing_converge());
    let worker = worker(intercept.clone());
    let before = intercept.surface();
    let error = worker.start_shared_owner().await.expect_err("shared rule convergence refuses");
    assert!(matches!(error, MtlsSharedOwnerError::Intercept { .. }));
    let addresses = intercept.listener_addresses();
    assert_eq!(addresses.len(), 2);
    for address in &addresses {
        let rebound =
            TcpListener::bind(*address).expect("failed start closes every partial socket");
        drop(rebound);
    }
    assert!(matches!(worker.audit_shared_owner().await, Err(MtlsSharedOwnerError::NotStarted)));
    worker.shutdown_owner().await.expect("Absent owner shutdown is idempotent");
    let after = intercept.surface();
    let mut expected = before;
    expected.call_counts = (2, 1, 1);
    expected.listener_addresses = addresses;
    assert_eq!(after, expected, "convergence refusal changes only exact bind/converge receipts");
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lost_leg_f_rebinds_the_recorded_nonzero_address_before_audit_succeeds() {
    let intercept = Arc::new(RecordingSharedIntercept::new());
    let worker = worker(intercept.clone());
    worker.start_shared_owner().await.expect("start the two-listener owner");
    let original = intercept.listener_addresses();
    assert_eq!(original.len(), 2);
    assert_ne!(original[0].port(), 0);
    intercept.terminate_listener_task(0);
    let failure = tokio::time::timeout(Duration::from_secs(2), worker.wait_shared_owner_failure())
        .await
        .expect("real listener task exit is observed");
    assert!(matches!(
        failure,
        MtlsSharedOwnerError::TaskReturned { leg: InterceptLeg::F }
            | MtlsSharedOwnerError::TaskFailed { leg: InterceptLeg::F, .. }
            | MtlsSharedOwnerError::TaskCancelled { leg: InterceptLeg::F }
    ));

    worker.converge_shared_owner().await.expect("exact-port recovery converges");
    worker.audit_shared_owner().await.expect("full post-recovery audit succeeds");
    let rebound = intercept.listener_addresses();
    assert_eq!(rebound.last(), Some(&original[0]), "recovery never selects another port");
    worker.shutdown_owner().await.expect("shared owner drains");
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn occupied_original_leg_f_address_refuses_recovery_without_selecting_another_port() {
    let intercept = Arc::new(RecordingSharedIntercept::new());
    let worker = worker(intercept.clone());
    worker.start_shared_owner().await.expect("start the two-listener owner");
    let original = intercept.listener_addresses();
    intercept.terminate_listener_task(0);
    let _ = tokio::time::timeout(Duration::from_secs(2), worker.wait_shared_owner_failure())
        .await
        .expect("listener failure is observed");
    intercept.occupy_next_exact_rebind();

    let error = worker
        .converge_shared_owner()
        .await
        .expect_err("EADDRINUSE on the recorded address refuses recovery");
    assert!(matches!(
        error,
        MtlsSharedOwnerError::ListenerBind {
            leg: InterceptLeg::F,
            requested,
            source: InterceptError::TransparentListener { source, .. },
        } if requested == original[0] && source.raw_os_error() == Some(libc::EADDRINUSE)
    ));
    assert_eq!(
        intercept.listener_addresses(),
        original,
        "failure does not bind a replacement address"
    );
    worker.shutdown_owner().await.expect("failed recovery still drains the owner tree");
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(
    clippy::too_many_lines,
    reason = "one published-owner narrative keeps exact targets, observe-only conflict, retained guard, and terminal drain together"
)]
async fn published_wrong_shared_target_is_observe_only_until_bounded_fail_stop() {
    let intercept = Arc::new(RecordingSharedIntercept::new());
    let worker = worker(intercept.clone());
    worker
        .start_shared_owner()
        .await
        .expect("publish two exact listeners, two tasks, and one node guard");
    worker.audit_shared_owner().await.expect("published owner begins healthy");

    let listener_addresses = intercept.listener_addresses();
    assert_eq!(listener_addresses.len(), 2);
    assert!(listener_addresses.iter().all(|address| address.port() != 0));
    let healthy_identity =
        intercept.shared_observation().expect("published owner retains one exact shared identity");
    let (expected_tables, expected_sets, expected_prerouting, expected_output) =
        SharedIpInterceptIdentity::for_listener_ports(
            listener_addresses[0].port(),
            listener_addresses[1].port(),
        )
        .expect("recorded non-zero listener addresses form one canonical identity")
        .normalized_parts();
    assert_eq!(
        healthy_identity,
        InterceptPostcondition::ConstantRules {
            table_and_chains: expected_tables,
            sets: expected_sets,
            prerouting: expected_prerouting,
            output: expected_output,
        },
        "published semantic identity names the two concrete listener targets"
    );
    let wrong_leg_f = SocketAddrV4::new(
        *listener_addresses[0].ip(),
        listener_addresses[0].port().checked_add(1).unwrap_or(1),
    );
    assert_ne!(wrong_leg_f, listener_addresses[0]);
    let (wrong_tables, wrong_sets, wrong_prerouting, wrong_output) =
        SharedIpInterceptIdentity::for_listener_ports(
            wrong_leg_f.port(),
            listener_addresses[1].port(),
        )
        .expect("different non-zero leg-F target remains canonical")
        .normalized_parts();
    let wrong_identity = InterceptPostcondition::ConstantRules {
        table_and_chains: wrong_tables,
        sets: wrong_sets,
        prerouting: wrong_prerouting,
        output: wrong_output,
    };
    intercept.replace_shared_observation(Some(wrong_identity.clone()));

    let calls_before_detection = intercept.call_counts();
    let detection = worker
        .audit_shared_owner()
        .await
        .expect_err("wrong published target is an immediate structured conflict");
    assert!(matches!(
        detection,
        MtlsSharedOwnerError::Intercept {
            source: InterceptError::PostconditionMismatch {
                expected,
                observed: Some(observed),
            },
        } if expected == healthy_identity && observed == wrong_identity
    ));

    let convergence_error = worker
        .converge_shared_owner()
        .await
        .expect_err("the production worker returns a present wrong-target conflict to its owner");
    assert!(matches!(
        convergence_error,
        MtlsSharedOwnerError::Intercept {
            source: InterceptError::PostconditionMismatch {
                expected: ref actual_expected,
                observed: Some(ref actual_observed),
            },
        } if actual_expected == &healthy_identity && actual_observed == &wrong_identity
    ));
    assert_eq!(
        intercept.listener_addresses(),
        listener_addresses,
        "the worker never binds port zero or substitutes an address"
    );
    assert_eq!(
        intercept.shared_observation().as_ref(),
        Some(&wrong_identity),
        "the worker never rewrites the published target"
    );
    assert_eq!(intercept.shared_guard_drops(), 0, "published guard stays retained");

    let calls_after_trigger = intercept.call_counts();
    assert_eq!(calls_after_trigger.0, calls_before_detection.0, "no runtime bind call");
    assert_eq!(
        calls_after_trigger.1, calls_before_detection.1,
        "runtime never calls the fresh-process converge_shared branch"
    );
    assert_eq!(
        calls_after_trigger.2,
        calls_before_detection.2 + 2,
        "detection plus the production worker convergence trigger are observe-only"
    );

    worker.shutdown_owner().await.expect("published owner drains after the bounded conflict");
    assert_eq!(
        intercept.shared_guard_drops(),
        0,
        "the private published-owner path relinquishes rather than drops the node guard"
    );
}

struct ActivationBarrierIntercept {
    shared: RecordingSharedIntercept,
    entered: AtomicBool,
    release: (StdMutex<bool>, Condvar),
    block_once: AtomicBool,
    guard_drops: Arc<AtomicUsize>,
    allocation_elements: Arc<Mutex<BTreeSet<SharedElement>>>,
    retired_elements: Arc<Mutex<BTreeSet<SharedElement>>>,
}

impl ActivationBarrierIntercept {
    fn new() -> Self {
        Self {
            shared: RecordingSharedIntercept::new(),
            entered: AtomicBool::new(false),
            release: (StdMutex::new(false), Condvar::new()),
            block_once: AtomicBool::new(true),
            guard_drops: Arc::new(AtomicUsize::new(0)),
            allocation_elements: Arc::new(Mutex::new(BTreeSet::new())),
            retired_elements: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    async fn wait_entered(&self) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !self.entered.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("allocation registration reaches the pre-activation barrier");
    }

    fn release(&self) {
        let (lock, wake) = &self.release;
        *lock.lock().expect("release lock") = true;
        wake.notify_all();
    }
}

impl MtlsIntercept for ActivationBarrierIntercept {
    fn bind_transparent(
        &self,
        addr: SocketAddrV4,
    ) -> overdrive_worker::mtls_intercept::Result<BoundListener> {
        self.shared.bind_transparent(addr)
    }

    fn converge_shared(
        &self,
        prior: Option<&InterceptPostcondition>,
        leg_f: SocketAddrV4,
        leg_c: SocketAddrV4,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        self.shared.converge_shared(prior, leg_f, leg_c)
    }

    fn observe_shared(
        &self,
    ) -> overdrive_worker::mtls_intercept::Result<Option<InterceptPostcondition>> {
        self.shared.observe_shared()
    }

    fn install_outbound(
        &self,
        source_addr: Ipv4Addr,
        _leg_f_port: u16,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        let owned = BTreeSet::from([
            SharedElement::ManagedGuest(source_addr),
            SharedElement::OutboundSource(source_addr),
        ]);
        self.allocation_elements.lock().extend(owned.iter().copied());
        Ok(Box::new(RecordingElementGuard {
            active: Arc::clone(&self.allocation_elements),
            retired: Arc::clone(&self.retired_elements),
            owned,
            drops: Arc::clone(&self.guard_drops),
        }))
    }

    fn install_inbound(
        &self,
        virt: SocketAddrV4,
        _leg_c_port: u16,
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
        let owned = BTreeSet::from([SharedElement::InboundDestination(virt)]);
        self.allocation_elements.lock().extend(owned.iter().copied());
        Ok(Box::new(RecordingElementGuard {
            active: Arc::clone(&self.allocation_elements),
            retired: Arc::clone(&self.retired_elements),
            owned,
            drops: Arc::clone(&self.guard_drops),
        }))
    }

    fn observe_shared_state(
        &self,
    ) -> overdrive_worker::mtls_intercept::Result<Option<InterceptState>> {
        Ok(self.shared.shared_observation.lock().clone().map(|program| InterceptState {
            program,
            policy_route: true,
            intercept_mark_guard: true,
            members: intercept_members(&self.allocation_elements.lock()),
        }))
    }

    fn converge_allocation_elements(
        &self,
        expected: &InterceptMembers,
    ) -> overdrive_worker::mtls_intercept::Result<Option<InterceptState>> {
        if self.shared.shared_observation.lock().clone().is_none() {
            return Ok(None);
        }
        *self.allocation_elements.lock() = member_elements(expected);
        self.observe_shared_state()
    }

    fn remove_allocation_elements(
        &self,
        source_addr: Ipv4Addr,
        destinations: &[SocketAddrV4],
    ) -> overdrive_worker::mtls_intercept::Result<InterceptState> {
        remove_elements(
            &self.allocation_elements,
            &self.retired_elements,
            source_addr,
            destinations,
        );
        self.observe_shared_state()?.ok_or_else(program_not_published)
    }
}

fn allocation_spec_at(name: &str, address: Ipv4Addr) -> AllocationSpec {
    let octet = address.octets()[3];
    AllocationSpec {
        alloc: AllocationId::new(name).expect("allocation id"),
        identity: SpiffeId::new(&format!(
            "spiffe://overdrive.local/workload/shared-owner/alloc/{name}"
        ))
        .expect("SPIFFE ID"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/bin/true".to_owned(),
            args: Vec::new(),
            kernel: "/kernel".into(),
            rootfs: "/rootfs".into(),
        }),
        resources: Resources { cpu_milli: 1, memory_bytes: 1 },
        probe_descriptors: Vec::new(),
        network: Some(GuestNetworkAssignment {
            address,
            tap: format!("ovd-tp-{octet:04}"),
            mac: [0x02, 0x00, 100, 95, 0, octet],
            gateway: Ipv4Addr::new(100, 95, 0, 1),
            prefix: 16,
            dns: Ipv4Addr::new(100, 95, 0, 1),
        }),
        service_ports: [8080_u16, 8443]
            .into_iter()
            .map(|port| std::num::NonZeroU16::new(port).expect("non-zero port"))
            .collect(),
    }
}

fn allocation_spec(name: &str) -> AllocationSpec {
    allocation_spec_at(name, Ipv4Addr::new(100, 95, 0, 2))
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_and_owner_shutdown_during_pending_registration_return_registration_retired_and_drain_once()
 {
    for owner_shutdown in [false, true] {
        let intercept = Arc::new(ActivationBarrierIntercept::new());
        let worker = worker(intercept.clone());
        worker.start_shared_owner().await.expect("shared owner is healthy");
        let owner_before = intercept.shared.surface();
        let spec = allocation_spec(if owner_shutdown {
            "pending-owner-shutdown"
        } else {
            "pending-allocation-stop"
        });
        let alloc = spec.alloc.clone();
        let start = tokio::spawn({
            let worker = Arc::clone(&worker);
            async move { worker.start_alloc(&spec).await }
        });
        intercept.wait_entered().await;
        let retirement = tokio::spawn({
            let worker = Arc::clone(&worker);
            let alloc = alloc.clone();
            async move {
                if owner_shutdown {
                    worker.shutdown_owner().await.map_err(|error| error.to_string())
                } else {
                    worker.stop_alloc(&alloc).await.map_err(|error| error.to_string())
                }
            }
        });
        tokio::task::yield_now().await;
        assert!(!retirement.is_finished(), "retirement waits for the Pending owner handoff");
        intercept.release();
        let start_error =
            start.await.expect("start task joins").expect_err("retirement wins before activation");
        assert!(matches!(
            start_error,
            overdrive_worker::mtls_intercept_worker::MtlsInterceptInstallError::RegistrationRetired {
                alloc_id,
            } if alloc_id == alloc
        ));
        retirement
            .await
            .expect("retirement task joins")
            .expect("one retirement owner drains every transferred effect");
        assert_eq!(
            intercept.guard_drops.load(Ordering::SeqCst),
            3,
            "one outbound group and two inbound tokens drop exactly once"
        );
        assert!(
            intercept.allocation_elements.lock().is_empty(),
            "all exact 2 + P set elements leave with the retired Pending registration"
        );
        let owner_after = intercept.shared.surface();
        assert_eq!(owner_after.call_counts, owner_before.call_counts);
        assert_eq!(owner_after.listener_addresses, owner_before.listener_addresses);
        assert_eq!(owner_after.observation, owner_before.observation);
        assert_eq!(owner_after.shared_guard_drops, 0);
        assert_eq!(worker.leg_c_addr(&alloc), None);
        if !owner_shutdown {
            worker.shutdown_owner().await.expect("remaining shared owner joins");
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EnforcementSurface {
    active: BTreeSet<EnforcedConnectionId>,
    torn_down: Vec<EnforcedConnectionId>,
    enforce_calls: usize,
}

struct RecordingSharedEnforcement {
    active: Mutex<BTreeSet<EnforcedConnectionId>>,
    torn_down: Mutex<Vec<EnforcedConnectionId>>,
    counter: std::sync::atomic::AtomicU64,
    enforce_calls: AtomicUsize,
    block_next: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

impl RecordingSharedEnforcement {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            active: Mutex::new(BTreeSet::new()),
            torn_down: Mutex::new(Vec::new()),
            counter: std::sync::atomic::AtomicU64::new(0),
            enforce_calls: AtomicUsize::new(0),
            block_next: AtomicBool::new(false),
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        })
    }

    fn surface(&self) -> EnforcementSurface {
        EnforcementSurface {
            active: self.active.lock().clone(),
            torn_down: self.torn_down.lock().clone(),
            enforce_calls: self.enforce_calls.load(Ordering::SeqCst),
        }
    }

    async fn wait_for_active(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while self.active.lock().len() != expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("production shared dispatch publishes the expected handles");
    }

    fn block_next_enforcement(&self) {
        self.block_next.store(true, Ordering::SeqCst);
    }

    async fn wait_blocked(&self) {
        self.entered.notified().await;
    }

    fn release_blocked(&self) {
        self.release.notify_one();
    }
}

#[async_trait::async_trait]
impl MtlsEnforcement for RecordingSharedEnforcement {
    async fn probe(&self) -> overdrive_core::traits::mtls_enforcement::Result<()> {
        Ok(())
    }

    async fn enforce(
        &self,
        connection: InterceptedConnection,
    ) -> overdrive_core::traits::mtls_enforcement::Result<EnforcedConnection> {
        self.enforce_calls.fetch_add(1, Ordering::SeqCst);
        if self.block_next.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        let sequence = self.counter.fetch_add(1, Ordering::SeqCst);
        let id = EnforcedConnectionId::new(connection.alloc, sequence);
        self.active.lock().insert(id.clone());
        Ok(EnforcedConnection::new(id))
    }

    fn liveness(&self, handle: &EnforcedConnection) -> PumpLiveness {
        if self.active.lock().contains(handle.id()) {
            PumpLiveness::Running
        } else {
            PumpLiveness::Gone
        }
    }

    async fn teardown(
        &self,
        handle: EnforcedConnection,
    ) -> overdrive_core::traits::mtls_enforcement::Result<()> {
        self.active.lock().remove(handle.id());
        self.torn_down.lock().push(handle.id().clone());
        Ok(())
    }
}

async fn connect_shared_source(source: Ipv4Addr, target: SocketAddrV4) -> tokio::net::TcpStream {
    let socket = tokio::net::TcpSocket::new_v4().expect("create shared-listener client socket");
    socket
        .bind(SocketAddrV4::new(source, 0).into())
        .expect("bind the allocation's immutable source address");
    socket.connect(target.into()).await.expect("connect through the shared leg-F listener")
}

async fn wait_listener_closed(address: SocketAddrV4) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if TcpListener::bind(address).is_ok() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owner shutdown closes the exact shared listener socket");
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_one_shared_allocation_preserves_the_unrelated_handle_and_complete_listener_owner()
{
    let intercept = Arc::new(RecordingSharedIntercept::new());
    let enforcement = RecordingSharedEnforcement::new();
    let worker = worker_with_enforcement(
        intercept.clone(),
        Arc::clone(&enforcement) as Arc<dyn MtlsEnforcement>,
    );
    worker.start_shared_owner().await.expect("publish exact F/C owner");
    let addresses = intercept.listener_addresses();
    let leg_f = addresses[0];
    let first = allocation_spec_at("shared-isolated-first", Ipv4Addr::new(127, 0, 0, 2));
    let second = allocation_spec_at("shared-isolated-second", Ipv4Addr::new(127, 0, 0, 3));
    worker.start_alloc(&first).await.expect("publish first capability");
    worker.start_alloc(&second).await.expect("publish second capability");
    let _first_client = connect_shared_source(Ipv4Addr::new(127, 0, 0, 2), leg_f).await;
    let _second_client = connect_shared_source(Ipv4Addr::new(127, 0, 0, 3), leg_f).await;
    enforcement.wait_for_active(2).await;

    let owner_before = intercept.surface();
    let enforcement_before = enforcement.surface();
    let unrelated_before = enforcement_before
        .active
        .iter()
        .find(|id| id.alloc() == &second.alloc)
        .expect("second allocation owns one handle")
        .clone();
    worker.stop_alloc(&first.alloc).await.expect("first exact capability drains");

    let owner_after = intercept.surface();
    let enforcement_after = enforcement.surface();
    let mut expected_owner = owner_before.clone();
    expected_owner.allocation_guard_drops += 3;
    expected_owner.allocation_elements = allocation_elements(&second);
    assert_eq!(owner_after, expected_owner, "only the first allocation elements change");
    assert_eq!(
        owner_before.allocation_elements,
        allocation_elements(&first).union(&allocation_elements(&second)).copied().collect(),
        "two active allocations own exactly their managed/source/destination elements"
    );
    assert_eq!(
        enforcement_after.active,
        BTreeSet::from([unrelated_before.clone()]),
        "the unrelated handle is byte-equal and remains live"
    );
    assert_eq!(enforcement_after.torn_down.len(), 1);
    assert_eq!(enforcement_after.torn_down[0].alloc(), &first.alloc);
    assert_eq!(worker.leg_c_addr(&second.alloc), Some(addresses[1]));
    worker.audit_shared_owner().await.expect("both listener tasks and node guard remain live");

    let _second_exchange = connect_shared_source(Ipv4Addr::new(127, 0, 0, 3), leg_f).await;
    enforcement.wait_for_active(2).await;
    worker.stop_alloc(&second.alloc).await.expect("second capability drains independently");
    intercept.release_listener_clones();
    worker.shutdown_owner().await.expect("shared owner joins");
    assert_eq!(intercept.shared_guard_drops(), 0);
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn owner_shutdown_waits_the_active_claim_then_drains_every_shared_capability_and_listener() {
    let intercept = Arc::new(RecordingSharedIntercept::new());
    let enforcement = RecordingSharedEnforcement::new();
    let worker = worker_with_enforcement(
        intercept.clone(),
        Arc::clone(&enforcement) as Arc<dyn MtlsEnforcement>,
    );
    worker.start_shared_owner().await.expect("publish exact F/C owner");
    let addresses = intercept.listener_addresses();
    let leg_f = addresses[0];
    let first = allocation_spec_at("shared-shutdown-first", Ipv4Addr::new(127, 0, 0, 4));
    let second = allocation_spec_at("shared-shutdown-second", Ipv4Addr::new(127, 0, 0, 5));
    worker.start_alloc(&first).await.expect("publish first capability");
    worker.start_alloc(&second).await.expect("publish second capability");
    let _first_client = connect_shared_source(Ipv4Addr::new(127, 0, 0, 4), leg_f).await;
    let _second_client = connect_shared_source(Ipv4Addr::new(127, 0, 0, 5), leg_f).await;
    enforcement.wait_for_active(2).await;

    enforcement.block_next_enforcement();
    let _held_client = connect_shared_source(Ipv4Addr::new(127, 0, 0, 5), leg_f).await;
    tokio::time::timeout(Duration::from_secs(2), enforcement.wait_blocked())
        .await
        .expect("third connection holds one immutable claim");
    let owner_before = intercept.surface();
    intercept.release_listener_clones();
    let mut shutdown = tokio::spawn({
        let worker = Arc::clone(&worker);
        async move { worker.shutdown_owner().await }
    });
    for address in &addresses {
        wait_listener_closed(*address).await;
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut shutdown).await.is_err(),
        "owner completion waits for the active claim"
    );
    enforcement.release_blocked();
    shutdown.await.expect("shutdown task joins").expect("every handle and capability drains");

    let final_enforcement = enforcement.surface();
    assert!(final_enforcement.active.is_empty());
    assert_eq!(final_enforcement.torn_down.len(), 3);
    assert_eq!(intercept.allocation_guard_drops(), 6);
    assert_eq!(intercept.shared_guard_drops(), 0, "node guard is privately relinquished");
    let owner_after = intercept.surface();
    assert_eq!(owner_after.call_counts, owner_before.call_counts);
    assert_eq!(owner_after.listener_addresses, owner_before.listener_addresses);
    assert_eq!(owner_after.observation, owner_before.observation);
    assert_eq!(owner_after.listener_clones, 0);
    assert!(owner_after.allocation_elements.is_empty());
}

// ---------------------------------------------------------------------------
// Listener test support (TS § "Intercept listener and stop-error test support")
// ---------------------------------------------------------------------------

/// An [`InterceptListener`] over a plain loopback socket bound at the requested
/// address.
///
/// - `bind` needs no Tokio runtime; the socket is registered with the reactor
///   on the first `accept`.
/// - `local_addr` reads the socket.
/// - `accept` is tokio's cancel-safe accept. It returns the accepted stream as a
///   blocking `OwnedFd`, the peer the accept reported, and `getsockname` as
///   `local`. Polled with no current runtime it returns `Accept` and never
///   panics.
/// - `lose()` shuts the listening socket down, so a pending or later `accept`
///   returns `Accept`.
///
/// `RecordingSharedIntercept::bind_transparent` returns this listener.
struct LoopbackInterceptListener {
    socket: Mutex<LoopbackSocket>,
    lost: AtomicBool,
}

enum LoopbackSocket {
    /// Bound and listening, not yet registered with a Tokio reactor.
    Bound(TcpListener),
    /// Registered by the first `accept`; shared with every in-flight accept.
    Registered(Arc<tokio::net::TcpListener>),
    /// Registration failed, which closed the socket.
    Closed,
}

impl LoopbackInterceptListener {
    fn bind(addr: SocketAddrV4) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            socket: Mutex::new(LoopbackSocket::Bound(listener)),
            lost: AtomicBool::new(false),
        })
    }

    /// Shut the listening socket down: a pending `accept` wakes with `Accept`,
    /// and every later `accept` returns `Accept`.
    fn lose(&self) {
        self.lost.store(true, Ordering::SeqCst);
        let socket = self.socket.lock();
        let fd = match &*socket {
            LoopbackSocket::Bound(listener) => listener.as_raw_fd(),
            LoopbackSocket::Registered(listener) => listener.as_raw_fd(),
            LoopbackSocket::Closed => return,
        };
        // SAFETY: `fd` belongs to the socket `socket` owns, and the guard is
        // held, so the descriptor stays open for this call. `shutdown` changes
        // socket state and takes no ownership.
        let status = unsafe { libc::shutdown(fd, libc::SHUT_RDWR) };
        let error = std::io::Error::last_os_error();
        drop(socket);
        assert_eq!(status, 0, "shutting the loopback listener down failed: {error}");
    }

    fn registered(&self) -> std::io::Result<Arc<tokio::net::TcpListener>> {
        let mut socket = self.socket.lock();
        let registered = match std::mem::replace(&mut *socket, LoopbackSocket::Closed) {
            LoopbackSocket::Bound(listener) => {
                Arc::new(tokio::net::TcpListener::from_std(listener)?)
            }
            LoopbackSocket::Registered(listener) => listener,
            LoopbackSocket::Closed => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "the loopback intercept listener socket is closed",
                ));
            }
        };
        *socket = LoopbackSocket::Registered(Arc::clone(&registered));
        drop(socket);
        Ok(registered)
    }
}

#[async_trait::async_trait]
impl InterceptListener for LoopbackInterceptListener {
    fn local_addr(&self) -> std::io::Result<SocketAddrV4> {
        let bound = match &*self.socket.lock() {
            LoopbackSocket::Bound(listener) => listener.local_addr()?,
            LoopbackSocket::Registered(listener) => listener.local_addr()?,
            LoopbackSocket::Closed => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "the loopback intercept listener socket is closed",
                ));
            }
        };
        match bound {
            SocketAddr::V4(bound) => Ok(bound),
            SocketAddr::V6(bound) => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("the loopback intercept listener is bound to IPv6 {bound}"),
            )),
        }
    }

    async fn accept(&self) -> Result<InterceptAccepted, InterceptAcceptError> {
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(InterceptAcceptError::Accept {
                source: std::io::Error::other(
                    "the loopback intercept listener was polled with no current Tokio runtime",
                ),
            });
        }
        if self.lost.load(Ordering::SeqCst) {
            return Err(InterceptAcceptError::Accept {
                source: std::io::Error::from_raw_os_error(libc::EINVAL),
            });
        }
        let listener =
            self.registered().map_err(|source| InterceptAcceptError::Accept { source })?;
        let (stream, peer) =
            listener.accept().await.map_err(|source| InterceptAcceptError::Accept { source })?;
        let stream = stream.into_std().map_err(|source| InterceptAcceptError::Accept { source })?;
        stream.set_nonblocking(false).map_err(|source| InterceptAcceptError::Accept { source })?;
        let SocketAddr::V4(peer) = peer else {
            unreachable!("an IPv4 loopback listener reports an IPv4 peer")
        };
        let local = match stream.local_addr() {
            Ok(SocketAddr::V4(local)) => local,
            Ok(SocketAddr::V6(local)) => {
                return Err(InterceptAcceptError::OriginalDestination {
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("the accepted connection's local address is IPv6 {local}"),
                    ),
                });
            }
            Err(source) => return Err(InterceptAcceptError::OriginalDestination { source }),
        };
        Ok(InterceptAccepted { stream: OwnedFd::from(stream), peer, local })
    }
}

// ---------------------------------------------------------------------------
// S-ND295-13D — the fresh-process member clear (D-295-R12 step 6.2)
// ---------------------------------------------------------------------------

/// The owned program and members a killed prior process left behind. Its
/// listener targets lie outside the default ephemeral range, so no listener
/// this process binds can share them.
fn prior_process_state() -> (InterceptPostcondition, BTreeSet<SharedElement>) {
    let program = RecordingSharedIntercept::identity(
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, 61_001),
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, 61_002),
    );
    let stale_guest = Ipv4Addr::new(100, 95, 0, 9);
    let members = BTreeSet::from([
        SharedElement::ManagedGuest(stale_guest),
        SharedElement::OutboundSource(stale_guest),
        SharedElement::InboundDestination(SocketAddrV4::new(stale_guest, 8080)),
    ]);
    (program, members)
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-13D — The fresh intercept owner refuses to start when stale members cannot be cleared.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER step 08-02 (S-ND295-13D)"]
async fn a_fresh_owner_clears_stale_members_before_reading_the_program() {
    let (stale_program, stale_members) = prior_process_state();
    let intercept = Arc::new(RecordingSharedIntercept::left_by_a_prior_process(
        stale_program.clone(),
        &stale_members,
    ));
    let worker = worker(intercept.clone());

    let started = worker.start_shared_owner().await;
    let journal = intercept.journal();
    assert_eq!(
        journal.first(),
        Some(&InterceptCall::ConvergeAllocationElements(InterceptMembers::default())),
        "the fresh owner converges the members to empty before any other port call, so before \
         it reads the program; journal: {journal:?}"
    );
    assert!(
        journal.iter().any(InterceptCall::reads_the_program),
        "the owner still reads the program after the clear; journal: {journal:?}"
    );
    started.expect("with the stale members cleared, the fresh owner starts");

    let addresses = intercept.listener_addresses();
    assert_eq!(addresses.len(), 2, "one leg-F and one leg-C listener");
    assert!(
        journal.contains(&InterceptCall::ConvergeShared {
            prior: Some(stale_program),
            leg_f: addresses[0],
            leg_c: addresses[1],
        }),
        "the prior identity read after the clear is the prior process's program; journal: \
         {journal:?}"
    );
    let state = intercept.kernel_state().expect("the owned program is present after start");
    assert_eq!(state.members, InterceptMembers::default(), "no stale member survives the start");
    assert_eq!(
        state.program,
        RecordingSharedIntercept::identity(addresses[0], addresses[1]),
        "the program names this process's listener targets"
    );
    worker.audit_shared_owner().await.expect("the published owner audits clean");
    worker.shutdown_owner().await.expect("the owner drains");
}

/// The worker-level half of G-295-1 row 2: a boot member clear that fails, or
/// that returns `Ok` with members left, refuses the fresh owner with
/// `BootMemberClear` whose source is the clear's own error or
/// `MembersRemain { observed }` naming the members left (FD § "[REF] Driven
/// port — intercept element release, member convergence, boot clear …" (the
/// typed causes of the observation checks, boot step 6.2)), binds no listener,
/// arms no node guard, and publishes nothing. It asserts no component:
/// `component()` lands at 08-03, and S-ND295-61's component table carries
/// `BootMemberClear`'s. The composed `run_server` evidence for rows 2 and 4
/// (`health.startup.refused`, EXEC BootClosed, a clear that commits after the
/// refusal publishing nothing) is not this body's: the worker holds no task
/// after a refused start, so nothing here could observe a late commit.
///
/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-13D — The fresh intercept owner refuses to start when stale members cannot be cleared.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER step 08-02 (S-ND295-13D)"]
async fn a_failed_member_clear_refuses_startup_without_publication() {
    for script in [
        ConvergeElementsScript::Refuse { errno: libc::EBUSY },
        ConvergeElementsScript::LeaveMembers,
    ] {
        let (stale_program, stale_members) = prior_process_state();
        let intercept = Arc::new(RecordingSharedIntercept::left_by_a_prior_process(
            stale_program.clone(),
            &stale_members,
        ));
        intercept.script_converge_elements(script);
        let worker = worker(intercept.clone());

        let Err(error) = worker.start_shared_owner().await else {
            panic!("{script:?}: a member clear that fails or leaves members must refuse the start");
        };
        match (script, &error) {
            (
                ConvergeElementsScript::Refuse { errno },
                MtlsSharedOwnerError::BootMemberClear {
                    source: InterceptError::NftRuleInstallFailed { op, source },
                },
            ) => {
                assert_eq!(*op, "converge-shared-members", "the clear's own refusal is the cause");
                assert_eq!(source.errno(), Some(-errno), "the cause keeps the clear's errno");
            }
            (
                ConvergeElementsScript::LeaveMembers,
                MtlsSharedOwnerError::BootMemberClear {
                    source: InterceptError::MembersRemain { observed },
                },
            ) => {
                assert_eq!(
                    *observed,
                    intercept_members(&stale_members),
                    "a clear that returns Ok with members left names exactly the stale members"
                );
            }
            (script, error) => {
                panic!(
                    "{script:?}: expected BootMemberClear carrying the clear's own error, or \
                     MembersRemain naming the members left, got {error:?}"
                )
            }
        }

        let journal = intercept.journal();
        assert_eq!(
            journal.first(),
            Some(&InterceptCall::ConvergeAllocationElements(InterceptMembers::default())),
            "{script:?}: the clear is the first port call; journal: {journal:?}"
        );
        assert!(
            !journal.iter().any(|call| matches!(
                call,
                InterceptCall::BindTransparent(_) | InterceptCall::ConvergeShared { .. }
            )),
            "{script:?}: a refused clear binds no listener and arms no node guard; journal: \
             {journal:?}"
        );
        assert!(intercept.listener_addresses().is_empty(), "{script:?}: no socket was bound");
        assert_eq!(intercept.recorded_identity(), None, "{script:?}: no node guard was armed");
        assert_eq!(intercept.shared_guard_drops(), 0, "{script:?}");
        assert!(
            matches!(worker.audit_shared_owner().await, Err(MtlsSharedOwnerError::NotStarted)),
            "{script:?}: a refused boot publishes nothing"
        );
        let kernel = intercept.kernel_state().expect("the prior process's program stays in place");
        assert_eq!(kernel.program, stale_program, "{script:?}: the refused boot writes no program");
        assert_eq!(
            kernel.members,
            intercept_members(&stale_members),
            "{script:?}: the refused clear changed no member"
        );
        worker.shutdown_owner().await.expect("an Absent owner's shutdown is idempotent");
        let journal = intercept.journal();
        assert!(
            !journal.iter().any(|call| matches!(
                call,
                InterceptCall::BindTransparent(_) | InterceptCall::ConvergeShared { .. }
            )),
            "{script:?}: shutting the refused owner down binds no listener and arms no guard; \
             journal: {journal:?}"
        );
        assert!(intercept.listener_addresses().is_empty(), "{script:?}");
        assert_eq!(intercept.recorded_identity(), None, "{script:?}");
    }
}

// ---------------------------------------------------------------------------
// S-ND295-61 — runtime member, policy-route, guard, and program audit and
// repair (D-295-R15)
// ---------------------------------------------------------------------------

/// The members the recording port holds for a set of live allocations: the
/// registry-expected set.
fn live_members(specs: &[&AllocationSpec]) -> InterceptMembers {
    let elements: BTreeSet<SharedElement> =
        specs.iter().flat_map(|spec| allocation_elements(spec)).collect();
    intercept_members(&elements)
}

/// The member calls in one slice of the journal.
fn member_convergences(calls: &[InterceptCall]) -> Vec<InterceptMembers> {
    calls
        .iter()
        .filter_map(|call| match call {
            InterceptCall::ConvergeAllocationElements(members) => Some(members.clone()),
            _ => None,
        })
        .collect()
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-61 — Lost members, policy route, or guard are detected within a second and repaired with live workloads.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER step 08-03 (S-ND295-61)"]
async fn member_loss_is_an_ipsets_failure_and_repair_restores_exactly_the_member() {
    let intercept = Arc::new(RecordingSharedIntercept::new());
    let worker = worker(intercept.clone());
    worker.start_shared_owner().await.expect("publish the shared owner");
    let first = allocation_spec_at("member-loss-first", Ipv4Addr::new(100, 95, 0, 2));
    let second = allocation_spec_at("member-loss-second", Ipv4Addr::new(100, 95, 0, 3));
    worker.start_alloc(&first).await.expect("admit the first allocation");
    worker.start_alloc(&second).await.expect("admit the second allocation");
    let registry_expected = live_members(&[&first, &second]);
    worker.audit_shared_owner().await.expect("the live members equal the registry");
    let healthy = intercept.kernel_state().expect("the owned program is present");
    assert_eq!(healthy.members, registry_expected, "baseline: the kernel holds every live member");

    let lost_destination = SocketAddrV4::new(Ipv4Addr::new(100, 95, 0, 2), 8443);
    intercept.lose_member(SharedElement::InboundDestination(lost_destination));
    let mut observed_after_loss = registry_expected.clone();
    assert!(observed_after_loss.inbound_destinations.remove(&lost_destination));

    let Err(detection) = worker.audit_shared_owner().await else {
        panic!("a member deleted from the kernel must fail the audit");
    };
    assert!(
        matches!(
            &detection,
            MtlsSharedOwnerError::MemberMismatch { expected, observed }
                if *expected == registry_expected && *observed == observed_after_loss
        ),
        "the audit reports the registry-expected and the observed members, got {detection:?}"
    );
    assert_eq!(detection.component(), SharedGuestNetworkComponent::IpSets);

    let repair_from = intercept.journal_len();
    let writes_before = intercept.program_writes();
    worker.converge_shared_owner().await.expect("the member repair converges");
    let repair = intercept.journal()[repair_from..].to_vec();
    assert_eq!(
        member_convergences(&repair),
        vec![registry_expected.clone()],
        "repair converges the members to exactly the registry-expected set; repair: {repair:?}"
    );
    assert_eq!(intercept.program_writes(), writes_before, "an intact program is not rewritten");
    let repaired = intercept.kernel_state().expect("the owned program is present");
    assert_eq!(repaired.members, registry_expected, "exactly the deleted member is restored");
    assert_eq!(repaired.program, healthy.program, "the program is unchanged");
    assert_eq!(intercept.shared_guard_drops(), 0, "the node guard is handed over, never dropped");
    worker.audit_shared_owner().await.expect("the repaired owner audits clean");

    worker.stop_alloc(&first.alloc).await.expect("the first allocation drains");
    worker.stop_alloc(&second.alloc).await.expect("the second allocation drains");
    worker.shutdown_owner().await.expect("the owner drains");
    assert_eq!(intercept.shared_guard_drops(), 0, "shutdown relinquishes the node guard");
}

/// One protection object the kernel can lose under an intact program.
#[derive(Debug, Clone, Copy)]
enum ProtectionLoss {
    /// The `fwmark 0x1 lookup 100` rule or table 100's local route.
    PolicyRoute,
    /// The D-295-R18 intercept-mark guard table.
    InterceptMarkGuard,
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-61 — Lost members, policy route, or guard are detected within a second and repaired with live workloads.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER step 08-03 (S-ND295-61)"]
#[allow(
    clippy::too_many_lines,
    reason = "one repair narrative keeps detection, the equal-identity re-converge, the guard handover, and the later admission together"
)]
async fn policy_route_loss_is_repaired_with_live_members_and_the_prior_guard_is_relinquished() {
    // The `InterceptMarkGuard` population is conditional on D-295-R18: step
    // 08-01 removes it if its native RED withdraws R18 (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the conditional parts: the E14 withdrawal conditions)).
    for (index, loss) in
        [ProtectionLoss::PolicyRoute, ProtectionLoss::InterceptMarkGuard].into_iter().enumerate()
    {
        let intercept = Arc::new(RecordingSharedIntercept::new());
        let worker = worker(intercept.clone());
        worker.start_shared_owner().await.expect("publish the shared owner");
        let addresses = intercept.listener_addresses();
        let recorded =
            intercept.recorded_identity().expect("the published owner recorded its identity");
        let live = allocation_spec_at(
            &format!("protection-loss-live-{index}"),
            Ipv4Addr::new(100, 95, 0, 2),
        );
        worker.start_alloc(&live).await.expect("admit a live allocation");
        let members = live_members(&[&live]);
        worker.audit_shared_owner().await.expect("baseline: the owner audits clean");

        match loss {
            ProtectionLoss::PolicyRoute => intercept.lose_policy_route(),
            ProtectionLoss::InterceptMarkGuard => intercept.lose_intercept_mark_guard(),
        }
        let Err(detection) = worker.audit_shared_owner().await else {
            panic!("{loss:?}: the loss must fail the audit");
        };
        // The audit's typed cause for each lost object (FD § "[REF] Driven port
        // — intercept element release, member convergence, boot clear …" (the
        // typed causes of the observation checks)): the program is intact, so
        // the policy-route check or the guard check is the first to fail.
        let cause_matches = match loss {
            ProtectionLoss::PolicyRoute => matches!(
                detection,
                MtlsSharedOwnerError::Intercept { source: InterceptError::PolicyRouteAbsent }
            ),
            ProtectionLoss::InterceptMarkGuard => matches!(
                detection,
                MtlsSharedOwnerError::Intercept {
                    source: InterceptError::InterceptMarkGuardAbsent
                }
            ),
        };
        assert!(
            cause_matches,
            "{loss:?}: the audit reports Intercept with the lost object's typed cause, got \
             {detection:?}"
        );
        assert_eq!(detection.component(), SharedGuestNetworkComponent::IpRules, "{loss:?}");

        let repair_from = intercept.journal_len();
        let writes_before = intercept.program_writes();
        worker
            .converge_shared_owner()
            .await
            .unwrap_or_else(|error| panic!("{loss:?}: the repair converges: {error}"));
        let repair = intercept.journal()[repair_from..].to_vec();
        assert!(
            repair.contains(&InterceptCall::ConvergeShared {
                prior: Some(recorded.clone()),
                leg_f: addresses[0],
                leg_c: addresses[1],
            }),
            "{loss:?}: repair re-converges the equal identity at the recorded targets; repair: \
             {repair:?}"
        );
        assert_eq!(
            member_convergences(&repair),
            vec![members.clone()],
            "{loss:?}: repair then converges the live members; repair: {repair:?}"
        );
        assert_eq!(intercept.program_writes(), writes_before, "{loss:?}: no program is written");
        assert_eq!(
            intercept.shared_guard_drops(),
            0,
            "{loss:?}: the prior node guard is relinquished, never dropped"
        );
        assert_eq!(
            intercept.recorded_identity(),
            Some(recorded.clone()),
            "{loss:?}: the recorded targets stay intact"
        );
        let repaired = intercept.kernel_state().expect("the owned program is present");
        assert!(repaired.policy_route, "{loss:?}: the policy route is present after repair");
        assert!(repaired.intercept_mark_guard, "{loss:?}: the guard table is present after repair");
        assert_eq!(repaired.program, recorded, "{loss:?}: the program is unchanged");
        assert_eq!(repaired.members, members, "{loss:?}: the live members survive the repair");
        worker
            .audit_shared_owner()
            .await
            .unwrap_or_else(|error| panic!("{loss:?}: the repaired owner audits clean: {error}"));

        let later = allocation_spec_at(
            &format!("protection-loss-later-{index}"),
            Ipv4Addr::new(100, 95, 0, 3),
        );
        worker
            .start_alloc(&later)
            .await
            .unwrap_or_else(|error| panic!("{loss:?}: a later allocation still installs: {error}"));
        assert_eq!(
            intercept.kernel_state().expect("the owned program is present").members,
            live_members(&[&live, &later]),
            "{loss:?}: the later allocation's members join the live ones"
        );

        worker.stop_alloc(&live.alloc).await.expect("the live allocation drains");
        worker.stop_alloc(&later.alloc).await.expect("the later allocation drains");
        worker.shutdown_owner().await.expect("the owner drains");
        assert_eq!(intercept.shared_guard_drops(), 0, "{loss:?}: shutdown relinquishes the guard");
    }
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-61 — Lost members, policy route, or guard are detected within a second and repaired with live workloads.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER step 08-03 (S-ND295-61)"]
#[allow(
    clippy::too_many_lines,
    reason = "the never-rewritten refusal and its absent-program control share one owner so the contrast is one population"
)]
async fn a_differently_targeted_program_is_never_rewritten() {
    let intercept = Arc::new(RecordingSharedIntercept::new());
    let worker = worker(intercept.clone());
    worker.start_shared_owner().await.expect("publish the shared owner");
    let addresses = intercept.listener_addresses();
    let recorded =
        intercept.recorded_identity().expect("the published owner recorded its identity");
    let live = allocation_spec_at("retargeted-live", Ipv4Addr::new(100, 95, 0, 2));
    worker.start_alloc(&live).await.expect("admit a live allocation");
    let members = live_members(&[&live]);
    worker.audit_shared_owner().await.expect("baseline: the owner audits clean");

    let wrong_leg_f =
        SocketAddrV4::new(*addresses[0].ip(), addresses[0].port().checked_add(1).unwrap_or(1));
    assert_ne!(wrong_leg_f, addresses[0]);
    let wrong = RecordingSharedIntercept::identity(wrong_leg_f, addresses[1]);
    intercept.replace_shared_observation(Some(wrong.clone()));

    let Err(detection) = worker.audit_shared_owner().await else {
        panic!("a differently targeted program must fail the audit");
    };
    assert!(
        matches!(
            &detection,
            MtlsSharedOwnerError::Intercept {
                source: InterceptError::PostconditionMismatch {
                    expected,
                    observed: Some(observed),
                },
            } if *expected == recorded && *observed == wrong
        ),
        "a program mismatch is Intercept over PostconditionMismatch naming the recorded and the \
         observed program, got {detection:?}"
    );
    assert_eq!(detection.component(), SharedGuestNetworkComponent::IpRules);

    let repair_from = intercept.journal_len();
    let writes_before = intercept.program_writes();
    let Err(refusal) = worker.converge_shared_owner().await else {
        panic!("a differently targeted program is refused, never repaired over");
    };
    assert!(
        matches!(
            &refusal,
            MtlsSharedOwnerError::Intercept {
                source: InterceptError::PostconditionMismatch {
                    expected,
                    observed: Some(observed),
                },
            } if *expected == recorded && *observed == wrong
        ),
        "the refusal names the recorded and the observed identity, got {refusal:?}"
    );
    assert_eq!(refusal.component(), SharedGuestNetworkComponent::IpRules);
    let repair = intercept.journal()[repair_from..].to_vec();
    assert!(
        !repair.iter().any(|call| matches!(call, InterceptCall::ConvergeShared { .. })),
        "the runtime never converges the program over a different identity; repair: {repair:?}"
    );
    assert_eq!(intercept.program_writes(), writes_before, "no program is written");
    let kernel = intercept.kernel_state().expect("the observed program is present");
    assert_eq!(kernel.program, wrong, "the differently targeted program stays as observed");
    assert_eq!(kernel.members, members, "no member is written over a different identity");
    assert_eq!(intercept.listener_addresses(), addresses, "no listener is rebound or retargeted");
    assert_eq!(intercept.shared_guard_drops(), 0, "the node guard stays retained");
    assert_eq!(intercept.recorded_identity(), Some(recorded.clone()));

    // Control on the same owner: once the whole table is deleted the program is
    // absent, not different, and repair recreates it at the recorded targets.
    intercept.delete_program();
    let Err(absent) = worker.audit_shared_owner().await else {
        panic!("an absent program must fail the audit");
    };
    assert!(
        matches!(
            &absent,
            MtlsSharedOwnerError::Intercept {
                source: InterceptError::PostconditionMismatch { expected, observed: None },
            } if *expected == recorded
        ),
        "an absent program is Intercept over PostconditionMismatch with nothing observed, got \
         {absent:?}"
    );
    assert_eq!(absent.component(), SharedGuestNetworkComponent::IpRules);
    let recreate_from = intercept.journal_len();
    worker.converge_shared_owner().await.expect("an absent program is recreated");
    let recreate = intercept.journal()[recreate_from..].to_vec();
    let created_at = recreate
        .iter()
        .position(|call| {
            *call
                == InterceptCall::ConvergeShared {
                    prior: None,
                    leg_f: addresses[0],
                    leg_c: addresses[1],
                }
        })
        .unwrap_or_else(|| {
            panic!("repair creates the program at the recorded targets; repair: {recreate:?}")
        });
    let restored_at = recreate
        .iter()
        .position(|call| *call == InterceptCall::ConvergeAllocationElements(members.clone()))
        .unwrap_or_else(|| panic!("repair restores the live members; repair: {recreate:?}"));
    assert!(created_at < restored_at, "the members return after the program; repair: {recreate:?}");
    let recreated = intercept.kernel_state().expect("the program is present again");
    assert_eq!(recreated.program, recorded, "the program names the recorded targets");
    assert_eq!(recreated.members, members, "the live members are restored");
    assert_eq!(intercept.shared_guard_drops(), 0, "the prior node guard is relinquished");
    assert_eq!(intercept.recorded_identity(), Some(recorded), "the recorded targets stay intact");
    worker.audit_shared_owner().await.expect("the recreated owner audits clean");

    worker.stop_alloc(&live.alloc).await.expect("the live allocation drains");
    worker.shutdown_owner().await.expect("the owner drains");
    assert_eq!(intercept.shared_guard_drops(), 0);
}
