//! Crate-private test support for this crate's source-local GH #295 bodies.
//!
//! Two pieces:
//!
//! - **Test-local ports** — `TestSharedOwner` (the `SharedGuestNetworkOwner`
//!   port with its inherited `GuestNetworkProvisioner` and `GuestStreamDial`),
//!   `TestGuestDnsFactory` and `TestGuestDns`. `overdrive-sim` depends on
//!   `overdrive-control-plane`, which dev-depends on `overdrive-sim`, so in a
//!   source-local test build the sim types implement the traits of a second
//!   compiled copy of this crate and do not coerce to `Arc<dyn crate::…>`
//!   (QPROJ-1, the sim-placement rule). These doubles stand in for the
//!   `overdrive-sim` doubles in this crate's own tests only.
//! - **The composed node** (`composed`, `integration-tests` lane) — one node
//!   booted the way `run_server` boots it (phase 1 over the seam, then
//!   `boot_server` with `GuestNetworkSource::Composed` over a `BootAdapters`
//!   bundle, NO-OVERRIDE-SEAMS) with the production guest-flow owner
//!   (`compose_guest_network` over `SimGuestFlowIo`), the production runtime
//!   shared-network supervisor, `SimMtlsIntercept` at the shared mTLS table
//!   and a `SimClock`. The Quint `quiescence` conformance driver and the
//!   recovery acceptance bodies (P6-S1..S6, C2) drive it; neither
//!   re-implements a rule of the owner or the supervisor.
//!
//! Their shape is test support (DISTILL's), not production API.

#![allow(
    clippy::option_if_let_else,
    dead_code,
    clippy::significant_drop_tightening,
    reason = "test support shared by several source-local bodies; each port call records and \
              decides under one state lock, so the journal order is the decision order"
)]

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddrV4;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use overdrive_core::guest_network::SharedGuestNetworkComponent;
use overdrive_core::guest_network::{GuestNetworkExecGate, GuestNetworkExecWiring};
use overdrive_core::guest_transport::dial::{
    DialPurpose, GuestDialError, GuestStream, GuestStreamDial,
};
use overdrive_core::guest_transport::{GUEST_NODE_GATEWAY, GUEST_NODE_PREFIX, GuestFlowPorts};
use overdrive_core::id::AllocationId;
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
use overdrive_core::vm::config::VsockPort;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
use overdrive_sim::adapters::{SimIdentityRead, SimMtlsIntercept, SimMtlsResolve};
use overdrive_worker::mtls_intercept_port::MtlsIntercept;
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;
use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::dns_responder::responder::{DnsResponderError, Result as DnsResult};
use crate::dns_responder::{GuestDns, GuestDnsDeps, GuestDnsFactory};
use crate::guest_network::{
    ForwardingActivation, ForwardingQuiescence, ForwardingRestore, GuestAddressPool,
    GuestNetworkError, GuestNetworkFact, GuestNetworkOperation, GuestNetworkPlan,
    GuestNetworkProvisioner, QuiescenceHold, QuiescenceLedger, QuiescenceRestoreError, Result,
    SharedGuestNetworkAudit, SharedGuestNetworkAuditError, SharedGuestNetworkOwner,
};

// ---------------------------------------------------------------------------
// TestSharedOwner
// ---------------------------------------------------------------------------

/// Outcome of one non-scripted-result port call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestCallOutcome {
    Ok,
    Failed,
}

/// Outcome of one `activate` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestActivateOutcome {
    /// `Ok(ForwardingActivation::Activated)`.
    Activated,
    /// `Ok(ForwardingActivation::QuiescenceLatched)`.
    QuiescenceLatched,
    /// `Ok(ForwardingActivation::ControlSessionPending)`.
    ControlSessionPending,
    /// `Err(PostconditionMismatch { operation: ForwardingActivate, .. })`:
    /// the CID is unknown or Retiring.
    Refused,
}

impl TestActivateOutcome {
    fn result(self, plan: &GuestNetworkPlan) -> Result<ForwardingActivation> {
        match self {
            Self::Activated => Ok(ForwardingActivation::Activated),
            Self::QuiescenceLatched => Ok(ForwardingActivation::QuiescenceLatched),
            Self::ControlSessionPending => Ok(ForwardingActivation::ControlSessionPending),
            Self::Refused => Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::ForwardingActivate,
                expected: GuestNetworkFact::AllocationForwarding {
                    alloc: plan.alloc().clone(),
                    cid: plan.assignment().cid(),
                    pairs: 0,
                },
                observed: None,
            }),
        }
    }
}

/// One port call, its allocation where it has one, and its outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestOwnerCall {
    Provision {
        alloc: AllocationId,
        outcome: TestCallOutcome,
    },
    Activate {
        alloc: AllocationId,
        outcome: TestActivateOutcome,
    },
    Teardown {
        alloc: AllocationId,
        outcome: TestCallOutcome,
    },
    ProbeStartup,
    SweepStale,
    ConvergeShared(TestCallOutcome),
    AuditShared(Option<SharedGuestNetworkComponent>),
    Dial {
        target: SocketAddrV4,
        purpose: DialPurpose,
    },
    /// `quiesce`, with the hold count it left.
    Quiesce {
        holds: u32,
    },
    /// `restore`, with its result and the hold count it left.
    Restore {
        result: std::result::Result<ForwardingRestore, QuiescenceRestoreError>,
        holds: u32,
    },
}

/// One journal entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestJournalEntry {
    pub call: TestOwnerCall,
}

#[derive(Debug, Default)]
struct TestOwnerState {
    /// Activation outcomes still to hand out, in order; empty → `Activated`.
    activations: VecDeque<TestActivateOutcome>,
    audit_failures: Vec<SharedGuestNetworkComponent>,
    journal: Vec<TestJournalEntry>,
}

/// Test-local shared guest-network owner: standing refusal slots, a scripted
/// sequence of activation outcomes, a counted quiescence over its own
/// `QuiescenceLedger` (D-UP-4, PD-2: public, so a test-local port issues and
/// redeems holds the way the production owner does), and a per-call journal.
///
/// Quiescence: `quiesce` issues a hold from this owner's ledger and raises
/// the count; `restore` redeems it at that ledger and lowers the count
/// (`Reopened` at zero, else `StillQuiesced`). A hold another ledger issued
/// is refused `Err(ForeignHold)` and changes nothing (the hold is dropped,
/// counted leaked at its issuing ledger). `script_foreign_restores` makes
/// this owner refuse its own holds as foreign — the composition defect the
/// supervisor's `ForeignHold` → `ForeignQuiescenceHold` mapping answers
/// (FHOLD-CHECK) — keeping each refused hold so it is not counted leaked.
/// It aborts no flow (it holds none): admission and dial effects of
/// quiescence are the production owner's, driven through [`composed`].
pub struct TestSharedOwner {
    provision_refused: AtomicBool,
    teardown_refused: AtomicBool,
    converge_refused: AtomicBool,
    foreign_restores: AtomicBool,
    ledger: QuiescenceLedger,
    holds: AtomicU32,
    refused_holds: Mutex<Vec<QuiescenceHold>>,
    /// PORTS-CONFIG: the ports its listeners "bound" — `None` until its
    /// first `converge_shared` returns `Ok`, then [`test_owner_flow_ports`]
    /// (the `SharedGuestNetworkOwner::flow_ports` postcondition the boot
    /// reads after step 9).
    flow_ports: Mutex<Option<GuestFlowPorts>>,
    state: Mutex<TestOwnerState>,
}

/// The ports a [`TestSharedOwner`] reports once its `converge_shared`
/// returned `Ok`. The double binds nothing, so these are fixture values in
/// the valid range, never a kernel's choice.
pub fn test_owner_flow_ports() -> GuestFlowPorts {
    GuestFlowPorts::new(VsockPort::new(50_240), VsockPort::new(50_243))
        .unwrap_or_else(|e| unreachable!("two distinct ports above the reserved range: {e}"))
}

impl Default for TestSharedOwner {
    fn default() -> Self {
        Self {
            provision_refused: AtomicBool::new(false),
            teardown_refused: AtomicBool::new(false),
            converge_refused: AtomicBool::new(false),
            foreign_restores: AtomicBool::new(false),
            ledger: QuiescenceLedger::new(),
            holds: AtomicU32::new(0),
            refused_holds: Mutex::new(Vec::new()),
            flow_ports: Mutex::new(None),
            state: Mutex::new(TestOwnerState::default()),
        }
    }
}

impl std::fmt::Debug for TestSharedOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestSharedOwner")
            .field("holds", &self.quiescence_holds())
            .field("leaked", &self.leaked_quiescence_holds())
            .finish_non_exhaustive()
    }
}

impl TestSharedOwner {
    /// An owner with every slot disarmed and `Activated` for every
    /// activation.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn script_provision_failure(&self, armed: bool) {
        self.provision_refused.store(armed, Ordering::SeqCst);
    }

    pub fn script_teardown_failure(&self, armed: bool) {
        self.teardown_refused.store(armed, Ordering::SeqCst);
    }

    pub fn script_converge_failure(&self, armed: bool) {
        self.converge_refused.store(armed, Ordering::SeqCst);
    }

    /// Armed: every `restore` answers `Err(ForeignHold)` as if the hold had
    /// been issued by another ledger, and keeps the hold (not leaked).
    pub fn script_foreign_restores(&self, armed: bool) {
        self.foreign_restores.store(armed, Ordering::SeqCst);
    }

    /// The outstanding hold count.
    pub fn quiescence_holds(&self) -> u32 {
        self.holds.load(Ordering::SeqCst)
    }

    /// Holds of this owner's ledger dropped unredeemed.
    pub fn leaked_quiescence_holds(&self) -> u32 {
        self.ledger.leaked()
    }

    /// The next activations return `outcomes`, in order; afterwards
    /// `Activated`.
    pub fn script_activations(&self, outcomes: impl IntoIterator<Item = TestActivateOutcome>) {
        self.state.lock().activations.extend(outcomes);
    }

    /// The node-level components the next audits report damaged, one per
    /// audit, in order.
    pub fn script_audit_failures(
        &self,
        components: impl IntoIterator<Item = SharedGuestNetworkComponent>,
    ) {
        self.state.lock().audit_failures.extend(components);
    }

    /// Every call, in call order; a non-draining snapshot.
    pub fn journal(&self) -> Vec<TestJournalEntry> {
        self.state.lock().journal.clone()
    }

    fn record(&self, call: TestOwnerCall) {
        self.state.lock().journal.push(TestJournalEntry { call });
    }

    fn outcome(armed: &AtomicBool) -> TestCallOutcome {
        if armed.load(Ordering::SeqCst) { TestCallOutcome::Failed } else { TestCallOutcome::Ok }
    }

    fn result(outcome: TestCallOutcome, operation: GuestNetworkOperation) -> Result<()> {
        match outcome {
            TestCallOutcome::Ok => Ok(()),
            TestCallOutcome::Failed => Err(GuestNetworkError::Io {
                operation,
                source: std::io::Error::other("scripted test owner refusal"),
            }),
        }
    }
}

#[async_trait::async_trait]
impl GuestNetworkProvisioner for TestSharedOwner {
    async fn provision(&self, plan: &GuestNetworkPlan) -> Result<()> {
        let outcome = Self::outcome(&self.provision_refused);
        self.record(TestOwnerCall::Provision { alloc: plan.alloc().clone(), outcome });
        Self::result(outcome, GuestNetworkOperation::PairAttach)
    }

    async fn activate(&self, plan: &GuestNetworkPlan) -> Result<ForwardingActivation> {
        let outcome = {
            let mut state = self.state.lock();
            let outcome = state.activations.pop_front().unwrap_or(TestActivateOutcome::Activated);
            state.journal.push(TestJournalEntry {
                call: TestOwnerCall::Activate { alloc: plan.alloc().clone(), outcome },
            });
            outcome
        };
        outcome.result(plan)
    }

    async fn teardown(&self, plan: &GuestNetworkPlan) -> Result<()> {
        let outcome = Self::outcome(&self.teardown_refused);
        self.record(TestOwnerCall::Teardown { alloc: plan.alloc().clone(), outcome });
        Self::result(outcome, GuestNetworkOperation::PairAbort)
    }
}

#[async_trait::async_trait]
impl GuestStreamDial for TestSharedOwner {
    /// The double serves no allocation: every dial is `NotServing` with no
    /// state change (the trait's "no Active VM allocation serves this
    /// address and port").
    async fn dial(
        &self,
        target: SocketAddrV4,
        purpose: DialPurpose,
    ) -> std::result::Result<GuestStream, GuestDialError> {
        self.record(TestOwnerCall::Dial { target, purpose });
        Err(GuestDialError::NotServing)
    }
}

#[async_trait::async_trait]
impl SharedGuestNetworkOwner for TestSharedOwner {
    async fn probe_startup(&self) -> Result<()> {
        self.record(TestOwnerCall::ProbeStartup);
        Ok(())
    }

    async fn sweep_stale(&self) -> Result<()> {
        self.record(TestOwnerCall::SweepStale);
        Ok(())
    }

    async fn converge_shared(&self) -> Result<()> {
        let outcome = Self::outcome(&self.converge_refused);
        self.record(TestOwnerCall::ConvergeShared(outcome));
        if outcome == TestCallOutcome::Ok {
            let mut bound = self.flow_ports.lock();
            if bound.is_none() {
                *bound = Some(test_owner_flow_ports());
            }
        }
        Self::result(outcome, GuestNetworkOperation::ForwarderOpen)
    }

    fn flow_ports(&self) -> Option<GuestFlowPorts> {
        *self.flow_ports.lock()
    }

    fn leaked_quiescence_holds(&self) -> u32 {
        self.ledger.leaked()
    }

    async fn audit_shared(
        &self,
    ) -> std::result::Result<SharedGuestNetworkAudit, SharedGuestNetworkAuditError> {
        let failed = {
            let mut state = self.state.lock();
            let failed = if state.audit_failures.is_empty() {
                None
            } else {
                Some(state.audit_failures.remove(0))
            };
            state.journal.push(TestJournalEntry { call: TestOwnerCall::AuditShared(failed) });
            failed
        };
        match failed {
            None => Ok(SharedGuestNetworkAudit::default()),
            Some(component) => Err(SharedGuestNetworkAuditError {
                component,
                source: GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::SharedAudit,
                    expected: GuestNetworkFact::SharedComponent { component, healthy: true },
                    observed: Some(GuestNetworkFact::SharedComponent { component, healthy: false }),
                },
            }),
        }
    }

    async fn quiesce(&self) -> (QuiescenceHold, ForwardingQuiescence) {
        let hold = self.ledger.issue();
        let holds = self.holds.fetch_add(1, Ordering::SeqCst) + 1;
        self.record(TestOwnerCall::Quiesce { holds });
        (hold, ForwardingQuiescence::default())
    }

    async fn restore(
        &self,
        hold: QuiescenceHold,
    ) -> std::result::Result<ForwardingRestore, QuiescenceRestoreError> {
        let result = if self.foreign_restores.load(Ordering::SeqCst) {
            self.refused_holds.lock().push(hold);
            Err(QuiescenceRestoreError::ForeignHold)
        } else {
            match self.ledger.redeem(hold) {
                Ok(()) => {
                    let left = self.holds.fetch_sub(1, Ordering::SeqCst) - 1;
                    Ok(if left == 0 {
                        ForwardingRestore::Reopened
                    } else {
                        ForwardingRestore::StillQuiesced
                    })
                }
                Err(foreign) => {
                    drop(foreign);
                    Err(QuiescenceRestoreError::ForeignHold)
                }
            }
        };
        self.record(TestOwnerCall::Restore { result, holds: self.holds.load(Ordering::SeqCst) });
        result
    }
}

// ---------------------------------------------------------------------------
// TestGuestDnsFactory / TestGuestDns
// ---------------------------------------------------------------------------

/// How a [`TestGuestDns`] serve future ends when the test ends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestGuestDnsServeExit {
    Return,
    Panic,
}

/// Test-local [`GuestDnsFactory`] with the pinned `SimGuestDnsFactory`
/// semantics: one standing probe slot shared by every responder it built, and
/// a build log in build order.
#[derive(Debug, Default)]
pub struct TestGuestDnsFactory {
    probe_refused: Arc<AtomicBool>,
    audits: Arc<TestAuditGate>,
    built: Mutex<Vec<Arc<TestGuestDns>>>,
}

/// The audit gate every responder of one factory shares: while held, each
/// `audit` call parks until the test releases it. The supervisor's full
/// audit awaits the DNS port's `audit` (the `Dns` component is audited in
/// every full audit, ADR-0124), so a held gate parks every detecting and
/// post-repair audit at a consistency point (QPROJ-6) — the forwarder's
/// `inventory()` read is synchronous (EVMOD-PORT) and cannot be parked.
#[derive(Debug)]
struct TestAuditGate {
    held: AtomicBool,
    pending: AtomicU32,
    permits: tokio::sync::Semaphore,
}

/// One audit parked at [`TestAuditGate::permits`]. The production supervisor
/// bounds an audit call with cancellation, so the test gate must retire its
/// pending count when that future is dropped as well as when it acquires a
/// permit normally.
struct PendingAudit<'a> {
    count: &'a AtomicU32,
}

impl<'a> PendingAudit<'a> {
    fn new(count: &'a AtomicU32) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Self { count }
    }
}

impl Drop for PendingAudit<'_> {
    fn drop(&mut self) {
        let previous = self.count.fetch_sub(1, Ordering::SeqCst);
        debug_assert!(previous > 0, "a pending DNS audit count cannot underflow");
    }
}

impl Default for TestAuditGate {
    fn default() -> Self {
        Self {
            held: AtomicBool::new(false),
            pending: AtomicU32::new(0),
            permits: tokio::sync::Semaphore::new(0),
        }
    }
}

impl TestGuestDnsFactory {
    pub fn script_probe_failure(&self, armed: bool) {
        self.probe_refused.store(armed, Ordering::SeqCst);
    }

    /// Park every later `audit` of this factory's responders until released.
    pub fn hold_audits(&self) {
        self.audits.held.store(true, Ordering::SeqCst);
    }

    /// The `audit` calls currently parked.
    pub fn pending_audits(&self) -> u32 {
        self.audits.pending.load(Ordering::SeqCst)
    }

    /// Release one parked `audit` (it then runs with its scripted outcome).
    pub fn release_audit(&self) {
        self.audits.permits.add_permits(1);
    }

    /// Every responder built, in build order; a non-draining snapshot.
    pub fn responders(&self) -> Vec<Arc<TestGuestDns>> {
        self.built.lock().clone()
    }
}

impl GuestDnsFactory for TestGuestDnsFactory {
    fn responder(&self, deps: GuestDnsDeps) -> Arc<dyn GuestDns> {
        drop(deps);
        let responder = Arc::new(TestGuestDns {
            probe_refused: Arc::clone(&self.probe_refused),
            audits: Arc::clone(&self.audits),
            audit_refused: AtomicBool::new(false),
            ending: Mutex::new(None),
            ended: Notify::new(),
        });
        self.built.lock().push(Arc::clone(&responder));
        responder
    }
}

/// Test-local [`GuestDns`] with the pinned `SimGuestDns` semantics.
#[derive(Debug)]
pub struct TestGuestDns {
    probe_refused: Arc<AtomicBool>,
    audits: Arc<TestAuditGate>,
    audit_refused: AtomicBool,
    ending: Mutex<Option<TestGuestDnsServeExit>>,
    ended: Notify,
}

impl TestGuestDns {
    /// Decide how `serve` ends, unless an earlier `end_serve` or `stop` did.
    pub fn end_serve(&self, exit: TestGuestDnsServeExit) {
        self.decide(exit);
    }

    /// Arm or disarm this responder's standing audit refusal.
    pub fn script_audit_failure(&self, armed: bool) {
        self.audit_refused.store(armed, Ordering::SeqCst);
    }

    fn decide(&self, exit: TestGuestDnsServeExit) {
        let mut ending = self.ending.lock();
        if ending.is_none() {
            *ending = Some(exit);
        }
        drop(ending);
        self.ended.notify_waiters();
    }
}

#[async_trait::async_trait]
impl GuestDns for TestGuestDns {
    async fn probe(&self) -> DnsResult<()> {
        if self.probe_refused.load(Ordering::SeqCst) {
            return Err(DnsResponderError::Probe {
                reason: "scripted test DNS probe refusal".to_owned(),
            });
        }
        Ok(())
    }

    #[allow(clippy::panic, reason = "a scripted serve panic is this double's contract")]
    async fn serve(self: Arc<Self>) {
        loop {
            let notified = self.ended.notified();
            let ending = *self.ending.lock();
            match ending {
                Some(TestGuestDnsServeExit::Return) => return,
                Some(TestGuestDnsServeExit::Panic) => panic!("scripted test DNS serve panic"),
                None => notified.await,
            }
        }
    }

    async fn audit(&self) -> DnsResult<()> {
        if self.audits.held.load(Ordering::SeqCst) {
            let pending = PendingAudit::new(&self.audits.pending);
            let permit = self.audits.permits.acquire().await;
            drop(pending);
            match permit {
                Ok(permit) => permit.forget(),
                Err(_closed) => unreachable!("the audit gate's semaphore is never closed"),
            }
        }
        if self.audit_refused.load(Ordering::SeqCst) {
            return Err(DnsResponderError::Socket {
                source: std::io::Error::other("scripted test DNS audit refusal"),
            });
        }
        Ok(())
    }

    fn stop(&self) {
        self.decide(TestGuestDnsServeExit::Return);
    }
}

// ---------------------------------------------------------------------------
// TestDataplaneComposer
// ---------------------------------------------------------------------------

/// Test-local `DataplaneComposer` (NO-OVERRIDE-SEAMS): the in-crate
/// stand-in for `overdrive-sim`'s `SimDataplaneComposer`, which implements
/// the trait of a second compiled copy of this crate in a source-local build
/// (module doc). `compose` returns a fresh `SimDataplane` and the node
/// address it was built with — `Ipv4Addr::LOCALHOST` for every composition
/// that injected `SimDataplane` before (the feature-delta's replacement
/// table) — or, when scripted, the `DataplaneBoot` refusal it was given.
/// It constructs nothing else and keeps no state across boots.
pub struct TestDataplaneComposer {
    host_ipv4: std::net::Ipv4Addr,
    refusal: Mutex<Option<crate::error::ControlPlaneError>>,
}

impl TestDataplaneComposer {
    /// Every `compose` succeeds over a fresh `SimDataplane`, reporting
    /// `host_ipv4` as the node's address.
    pub fn new(host_ipv4: std::net::Ipv4Addr) -> Self {
        Self { host_ipv4, refusal: Mutex::new(None) }
    }

    /// The next `compose` returns `refusal` (one of the boot's
    /// `DataplaneBoot` refusals) and constructs nothing.
    pub fn script_refusal(&self, refusal: crate::error::ControlPlaneError) {
        *self.refusal.lock() = Some(refusal);
    }
}

#[async_trait::async_trait]
impl crate::DataplaneComposer for TestDataplaneComposer {
    async fn compose(
        &self,
        _plan: crate::DataplanePlan<'_>,
    ) -> std::result::Result<crate::ComposedDataplane, crate::error::ControlPlaneError> {
        let scripted = self.refusal.lock().take();
        if let Some(refusal) = scripted {
            return Err(refusal);
        }
        Ok(crate::ComposedDataplane {
            dataplane: Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
            host_ipv4: self.host_ipv4,
        })
    }
}

// ---------------------------------------------------------------------------
// Fixture constructors for source-local `AppState` builds
// ---------------------------------------------------------------------------

pub fn mtls_worker() -> Arc<MtlsInterceptWorker> {
    worker_over(Arc::new(SimMtlsIntercept::new()), Arc::new(SimClock::new()))
}

/// An unstarted mTLS worker over `intercept`, with sim enforcement (its leg-S
/// guest dial a `SimGuestStreamDial`, LEGS-DIAL) and a non-mesh resolver.
pub fn worker_over(
    intercept: Arc<SimMtlsIntercept>,
    clock: Arc<SimClock>,
) -> Arc<MtlsInterceptWorker> {
    let enforcement: Arc<dyn MtlsEnforcement> = Arc::new(SimMtlsEnforcement::new(
        Arc::new(SimIdentityRead::new(BTreeMap::new(), None)),
        MtlsLimits::default(),
        Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
    ));
    let resolve: Arc<dyn MtlsResolve> =
        Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh));
    Arc::new(MtlsInterceptWorker::new(
        enforcement,
        resolve,
        clock,
        intercept as Arc<dyn MtlsIntercept>,
    ))
}

pub fn shared_guest_network_owner() -> Arc<TestSharedOwner> {
    Arc::new(TestSharedOwner::new())
}

pub fn guest_network_exec_gate() -> Arc<GuestNetworkExecGate> {
    GuestNetworkExecWiring::new(Arc::new(SimClock::new())).gate()
}

/// A pool over the production node prefix and gateway with seeded entropy
/// (DG-2 row 1: `new(node_prefix, gateway, Arc<dyn Entropy>)`).
pub fn guest_pool() -> Arc<GuestAddressPool> {
    Arc::new(GuestAddressPool::new(
        GUEST_NODE_PREFIX,
        GUEST_NODE_GATEWAY,
        Arc::new(SimEntropy::new(0x295)),
    ))
}

// ---------------------------------------------------------------------------
// The composed node (integration-tests lane)
// ---------------------------------------------------------------------------

#[cfg(feature = "integration-tests")]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "a test rig fails at once on a broken precondition, naming it"
)]
pub mod composed {
    //! One node booted through the production composition with the guest-flow
    //! owner over `SimGuestFlowIo` (QPROJ-1: `quiescence` — the host owner
    //! with the runtime shared-network supervisor; boot step 8 through
    //! `SimMtlsIntercept`).
    //!
    //! **What is production.** The composed boot (T-8, D-8): phase 1
    //! (`probe_guest_flow_startup` over the process's seam, before any boot
    //! effect, refusing `guest_network.probe`), then `boot_server` with
    //! `GuestNetworkSource::Composed` — boot order, phase 2's
    //! `compose_guest_network` (the owner and its `MtlsInterceptWorker` in one
    //! construction, the boot's one `OwnerDial` bound to the owner and given
    //! to the inbound enforcement as its leg-S dial, LEGS-DIAL), the step-8
    //! refusal of ADR-0184, the supervisor spawn and `open_after_boot` — the
    //! runtime shared-network supervisor, the `HostSharedGuestNetworkOwner`,
    //! the `GuestAddressPool`. The rig reaches the composed owner through the
    //! boot's `OwnerDial::owner()` (D-13), the pinned binding accessor.
    //!
    //! **What is simulated.** The owner's effect seam (`SimGuestFlowIo` over a
    //! `SimGuestNode` whose node-global objects outlive a process), the shared
    //! mTLS table (`SimMtlsIntercept`, the QPROJ-7 arbiter), the clock
    //! (`SimClock`), the dataplane (`TestDataplaneComposer` over
    //! `SimDataplane`), the enforcement (`SimMtlsEnforcementFactory`), the
    //! KEK, entropy, the workflow transport, DNS, cgroupfs, the observation
    //! store and VM host state. Each process gets a fresh seam; the shared
    //! table (the only node-global object the bodies read) outlives the
    //! process.
    //!
    //! **The adapters bundle (NO-OVERRIDE-SEAMS, STORES-IN).** Every boot is
    //! handed a fresh `BootAdapters` built here — the rig's driven adapters,
    //! no `ServerConfig` adapter field (none exists). The node's persistent
    //! stores are the real ones over the node's data directory, reopened from
    //! disk at every boot with `StoreAdapters::open` after the previous
    //! process released them (`kill_for_test` drops every handle), never a
    //! clone of a previous boot's handles, so a restart reads the node's
    //! durable state back the way a new process does. The workload-identity
    //! CA comes from a fresh `CaAdapters::host()` per boot; the boot builds it
    //! over the boot's seeded entropy and runs its generate-or-adopt over the
    //! reopened intent store.
    //!
    //! **Stepping.** Every handle the rig hands back is driven on a
    //! current-thread tokio runtime with paused time; `settle` resolves when
    //! no task of the runtime is ready (`SimGuestFlowIo::settled`). Time the
    //! owner and the supervisor read advances only through
    //! [`ComposedNode::advance`].
    //!
    //! **Supervisor stepping.** After boot the rig holds two effect kinds at
    //! the owner seam: `BindListener` (the repair of a listener component:
    //! `converge_shared` calls `bind_listener` on the held socket, which
    //! confirms it still listens and re-spawns accept — no close, no bind,
    //! round-11 "Repair") and `OpenForwarder` (the repair of the
    //! `Forwarder` component: the seam's `open_forwarder`, after
    //! `close_forwarder`, EVMOD-CTL); and it holds the process's DNS audit
    //! gate (`TestGuestDnsFactory::hold_audits`): every full audit awaits the
    //! `Dns` component's `GuestDns::audit` (ADR-0124), while the forwarder's
    //! `inventory()` is a synchronous port call that cannot be parked
    //! (EVMOD-PORT). A recovery therefore parks at its repair and at its
    //! post-repair audit, which is
    //! what lets a step observe `RecoveryPhase::Repaired` between the repair
    //! and the restore (QPROJ-6 consistency point). Every other effect is
    //! released as it is issued. The seam's issued-effect journal
    //! (`SimGuestFlowIo::issued`) records the synchronous forwarder calls
    //! (`abort_all`, `close_forwarder`) in call order beside the held ones.
    //!
    //! **Defect stimuli (QHOLD, no new seam).** `leak_hold` is the model's
    //! `LeakHold`: the composed owner's own `quiesce()`, the hold dropped
    //! unredeemed (D-UP-3). `restore_foreign_hold` is `ForeignRestore`: a hold
    //! issued by a second, public `QuiescenceLedger` (PD-2) handed to the
    //! composed owner's `restore` (D-UP-4).
    //!
    //! **Identity relabelling** (QPROJ-1): the model's component `C1` ↦
    //! `SharedGuestNetworkComponent::FlowListeners` (the stream listener's
    //! accept task exits), `C2` ↦ `ControlListener` (the control listener's
    //! accept task exits).
    //!
    //! **The owner's mTLS port.** The composed boot builds the owner and the
    //! worker together (D-UP-16's postcondition holds in the rig): the
    //! owner's `GuestFlowOwnerPorts.mtls` is the worker `serve` runs, over
    //! the node's `SimMtlsIntercept`.

    use std::io;
    use std::net::{Ipv4Addr, SocketAddrV4};
    use std::sync::Arc;
    use std::time::Duration;

    use overdrive_core::guest_network::{
        GuestNetworkExecWiring, ServeShutdownRequest, SharedGuestNetworkComponent,
    };
    use overdrive_core::guest_transport::wire::{ControlOpen, FlowId, FlowOrigin, FlowRequest};
    use overdrive_core::guest_transport::{
        GUEST_NODE_GATEWAY, GUEST_NODE_PREFIX, GuestCid, GuestFlowPorts, LaunchToken,
        NodeVsockPortConfig,
    };
    use overdrive_core::id::{AllocationId, NodeId};
    use overdrive_core::net::ListenPort;
    use overdrive_core::traits::driver::{DriverRegistry, DriverType};
    use overdrive_core::vm::config::VsockPort;
    use overdrive_sim::adapters::clock::SimClock;
    use overdrive_sim::adapters::driver::SimDriver;
    use overdrive_sim::adapters::entropy::SimEntropy;
    use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcementFactory;
    use overdrive_sim::adapters::observation_store::SimObservationStore;
    use overdrive_sim::adapters::transport::SimTransport;
    use overdrive_sim::adapters::{SimKek, SimMtlsIntercept, SimSharedTable, SimVmHostState};
    use tempfile::TempDir;

    use super::{TestDataplaneComposer, TestGuestDnsFactory, TestSharedOwner};
    use crate::error::ControlPlaneError;
    use crate::guest_network::flow_io::sim::{
        SimEffectId, SimEffectKind, SimEffectTarget, SimFailure, SimForwarderFailure,
        SimGuestFlowIo,
    };
    use crate::guest_network::flow_io::{FlowListener, GuestFlowIo};
    use crate::guest_network::{
        ForwardingActivation, ForwardingRestore, GuestAddressPool, GuestConnKey, GuestNetworkPlan,
        GuestNetworkProvisioner, GuestOwnerObservation, HostSharedGuestNetworkOwner,
        QuiescenceLedger, QuiescenceRestoreError, SharedGuestNetworkOwner as _,
    };
    use crate::guest_network_composition::OwnerDial;
    use crate::{
        BootAdapters, CaAdapters, MtlsAdapters, RecoveryEpisode, ServerConfig, ServerHandle,
        StoreAdapters, run_server_with_obs_and_drivers,
    };
    use overdrive_core::guest_transport::forwarder::ForwarderOp;
    use overdrive_netlink::FibRouteType;

    /// The seam seed of a node's first boot; each later boot adds its boot
    /// number, so every process of a node draws its own accept order.
    const SEAM_SEED: u64 = 0x0295_0169;

    /// The model's component `C1`.
    pub const C1: SharedGuestNetworkComponent = SharedGuestNetworkComponent::FlowListeners;
    /// The model's component `C2`.
    pub const C2: SharedGuestNetworkComponent = SharedGuestNetworkComponent::ControlListener;

    /// The flow listener whose accept task is the component's task.
    pub const fn listener_of(component: SharedGuestNetworkComponent) -> FlowListener {
        match component {
            SharedGuestNetworkComponent::ControlListener => FlowListener::Control,
            _ => FlowListener::Stream,
        }
    }

    /// How the last `serve` process ended, as the caller observed it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum NodeStop {
        /// The process was dropped (`kill_for_test`).
        Crash,
        /// The supervisor requested `ServeShutdownRequest::SharedGuestNetwork`
        /// and the process then ended.
        FailStop,
        /// The boot (`boot_server`) returned a typed boot refusal.
        Refused,
    }

    /// The non-mesh destination every guest-opened flow of the rig dials.
    pub const FLOW_DESTINATION: SocketAddrV4 =
        SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 7), 443);

    /// The declared listen port of the rig's allocation.
    pub const LISTEN_PORT: u16 = 8080;

    /// A live `serve` process of the node.
    pub struct Process {
        pub io: Arc<SimGuestFlowIo>,
        /// The composed owner, read once after boot through the boot's
        /// `OwnerDial::owner()` (D-13).
        pub owner: Arc<HostSharedGuestNetworkOwner>,
        /// The boot's one `OwnerDial`, bound to `owner` by phase 2.
        pub dial: Arc<OwnerDial>,
        pub server: ServerHandle,
        /// The process's DNS factory: its responders' `audit` is the rig's
        /// supervisor-stepping point (module doc).
        dns: Arc<TestGuestDnsFactory>,
        /// The Active allocation the rig opened, with its plan and the guest
        /// source port of its next flow.
        cid: Option<ActiveCid>,
    }

    struct ActiveCid {
        plan: GuestNetworkPlan,
        next_peer_port: u32,
        next_flow_seq: u32,
    }

    /// One node: the node-global state (the shared table, the data
    /// directory) and at most one live process.
    pub struct ComposedNode {
        pub clock: Arc<SimClock>,
        pub intercept: Arc<SimMtlsIntercept>,
        /// The pool the rig's plans come from (`GuestAddressPool::assign`
        /// is crate-private; the server's own pool is not reachable from a
        /// `ServerHandle`). Its plans name the production prefix and gateway.
        pub pool: Arc<GuestAddressPool>,
        pub process: Option<Process>,
        pub last_stop: Option<NodeStop>,
        /// A scripted outcome for the next boot's forwarder probe (boot
        /// phase 1, `probe_forwarder`: the probe context's open and its
        /// stages; D-8, UI-COV-1).
        next_forwarder_open: Option<SimFailure>,
        /// The seam of the last refused boot (its issued effects stay
        /// readable).
        refused_io: Option<Arc<SimGuestFlowIo>>,
        /// A fail-stop request already observed non-blockingly by the
        /// fixture. Ending that same process consumes this cached request
        /// instead of awaiting the one-shot receiver a second time.
        fail_stop_request: Option<ServeShutdownRequest>,
        boots: u32,
        dirs: TempDir,
    }

    impl ComposedNode {
        /// A node that has never booted (the model's `InitDown`): no table,
        /// no process.
        pub fn fresh() -> Self {
            Self {
                clock: Arc::new(SimClock::new()),
                intercept: Arc::new(SimMtlsIntercept::new()),
                pool: Arc::new(GuestAddressPool::new(
                    GUEST_NODE_PREFIX,
                    GUEST_NODE_GATEWAY,
                    Arc::new(SimEntropy::new(0x0295_0169)),
                )),
                process: None,
                last_stop: Some(NodeStop::Crash),
                next_forwarder_open: None,
                refused_io: None,
                fail_stop_request: None,
                boots: 0,
                dirs: TempDir::new().expect("node tempdir"),
            }
        }

        /// The next boot's phase-1 forwarder probe (`probe_forwarder`, which
        /// opens a context, runs its stages and releases it) fails with
        /// `failure` (`DeviceAbsent`, `AbiMismatch` or a
        /// probe stage); the boot refuses before any boot effect (D-8).
        pub fn fail_next_forwarder_open(&mut self, failure: SimFailure) {
            self.next_forwarder_open = Some(failure);
        }

        /// The process, which a step that needs one requires.
        pub fn process(&self) -> &Process {
            self.process.as_ref().expect("the node has a live serve process")
        }

        fn process_mut(&mut self) -> &mut Process {
            self.process.as_mut().expect("the node has a live serve process")
        }

        /// One composed boot over a fresh seam (T-8, D-8): phase 1
        /// (`probe_guest_flow_startup`, the forwarder probe included) over the
        /// seam, then `boot_server` with `GuestNetworkSource::Composed`,
        /// whose step 5 composes the owner and the worker and binds the
        /// boot's `OwnerDial`. `Ok` → the process runs; `Err` → the typed
        /// boot refusal, no process, `last_stop = Refused`.
        pub async fn boot(&mut self) -> std::result::Result<(), ControlPlaneError> {
            match self.boot_with_deferred_refusal_exit().await {
                Ok(()) => Ok(()),
                Err((refusal, io)) => {
                    io.exit_process();
                    Err(refusal)
                }
            }
        }

        /// Boot one process while deferring a refused process's seam exit to
        /// a fixture that owns and must first destroy that process runtime.
        /// Successful boot has the same retained [`Process`] as [`Self::boot`].
        pub async fn boot_with_deferred_refusal_exit(
            &mut self,
        ) -> std::result::Result<(), (ControlPlaneError, Arc<SimGuestFlowIo>)> {
            assert!(self.process.is_none(), "boot needs a stopped node");
            self.fail_stop_request = None;
            self.boots += 1;
            // A fresh seam per process: every effect auto-released, accepts
            // and record bytes delivered at once (the seam's defaults); the
            // accept order is drawn from a per-boot seed (D-UP-19).
            let io = SimGuestFlowIo::new(SEAM_SEED + u64::from(self.boots));
            io.auto_release(SimEffectKind::ProbeForwarder);
            if let Some(failure) = self.next_forwarder_open.take() {
                io.script_failure(SimEffectKind::ProbeForwarder, failure);
            }
            let dns = Arc::new(TestGuestDnsFactory::default());
            let config = self.config();
            let dial = Arc::new(OwnerDial::unbound());
            let booted = self.boot_composed(config, Arc::clone(&dns), &io, &dial).await;
            match booted {
                Ok(server) => {
                    io.settled().await;
                    let owner = dial
                        .owner()
                        .expect("phase 2 binds the boot's OwnerDial to the composed owner");
                    // Supervisor stepping: park repairs and audits (module doc).
                    io.hold(SimEffectKind::BindListener);
                    io.hold(SimEffectKind::OpenForwarder);
                    dns.hold_audits();
                    self.process = Some(Process { io, owner, dial, server, dns, cid: None });
                    self.last_stop = None;
                    Ok(())
                }
                Err(refusal) => {
                    self.refused_io = Some(Arc::clone(&io));
                    self.last_stop = Some(NodeStop::Refused);
                    Err((refusal, io))
                }
            }
        }

        /// The composed boot's two phases over `io` and `dial`, with a fresh
        /// adapters bundle (NO-OVERRIDE-SEAMS): `boot_server` composes over
        /// it and constructs no driven adapter (STORES-IN).
        async fn boot_composed(
            &self,
            config: ServerConfig,
            dns: Arc<TestGuestDnsFactory>,
            io: &Arc<SimGuestFlowIo>,
            dial: &Arc<OwnerDial>,
        ) -> std::result::Result<ServerHandle, ControlPlaneError> {
            // The composition root's bundle first, as `overdrive serve` builds
            // it before `run_server` (NodeAdapters::host opens the stores).
            let boot = self.boot_adapters(&config, dns)?;
            // Phase 1 (D-8): the production boot-step-1 call `run_server`
            // makes before any boot effect — `probe_guest_flow_startup` over
            // the boot's seam, the boot's VIP range and `GUEST_NODE_PREFIX`,
            // with its `health.startup.refused` (`guest_network.probe`). A
            // refusal drops the bundle, which closes the stores.
            crate::probe_guest_flow_boot(io.as_ref(), &config.vip_range).await?;
            let mut registry = DriverRegistry::new();
            registry.insert(Arc::new(SimDriver::new(DriverType::Vm)));
            crate::boot_server(
                config,
                boot,
                Arc::new(registry),
                crate::GuestNetworkSource::Composed {
                    io: Arc::clone(io) as Arc<dyn GuestFlowIo>,
                    dial: Arc::clone(dial),
                },
                GuestNetworkExecWiring::new(self.clock.clone()),
            )
            .await
        }

        /// The node's configuration: configuration only (NO-OVERRIDE-SEAMS),
        /// over the node's directories, the vsock listeners kernel-assigned
        /// (PORTS-CONFIG; the owner reports what it bound, `flow_ports()`).
        fn config(&self) -> ServerConfig {
            let data_dir = self.dirs.path().join("data");
            let operator_config_dir = self.dirs.path().join("conf");
            std::fs::create_dir_all(&data_dir).expect("create data dir");
            std::fs::create_dir_all(&operator_config_dir).expect("create config dir");
            ServerConfig {
                data_dir,
                operator_config_dir,
                vsock_ports: NodeVsockPortConfig::kernel_assigned(),
                ..ServerConfig::new()
            }
        }

        /// One boot's driven adapters (NO-OVERRIDE-SEAMS, STORES-IN): the
        /// rig's one clock, seeded entropy, the simulated workflow
        /// transport, the boot KEK, a fresh simulated observation store and
        /// VM host state, the node's persistent stores reopened from
        /// `config.data_dir` (the previous process, if any, released them),
        /// a fresh workload-identity CA, the loopback dataplane composer, the
        /// node's `SimMtlsIntercept` with the simulated enforcement factory
        /// (built by the boot over its one `OwnerDial`, LEGS-DIAL), and
        /// `dns`. Err: the store open's typed refusal, as the boot returns it.
        fn boot_adapters(
            &self,
            config: &ServerConfig,
            dns: Arc<TestGuestDnsFactory>,
        ) -> std::result::Result<BootAdapters, ControlPlaneError> {
            let entropy = Arc::new(SimEntropy::new(SEAM_SEED + u64::from(self.boots)));
            let stores = StoreAdapters::open(config)?;
            Ok(BootAdapters {
                clock: self.clock.clone(),
                entropy,
                transport: Arc::new(SimTransport::new()),
                kek: Arc::new(SimKek::for_boot()),
                observation: Arc::new(SimObservationStore::single_peer(node_id(), 0)),
                vm_host_state: Arc::new(SimVmHostState::new()),
                stores,
                ca: CaAdapters::host(),
                dataplane: Arc::new(TestDataplaneComposer::new(Ipv4Addr::LOCALHOST)),
                mtls: MtlsAdapters {
                    intercept: Arc::clone(&self.intercept) as _,
                    enforcement: Arc::new(SimMtlsEnforcementFactory::new()),
                },
                guest_dns: dns,
            })
        }

        /// The ports the live process's owner bound (PORTS-CONFIG): `Some`
        /// from its first `Ok` `converge_shared`, which the boot ran.
        pub fn flow_ports(&self) -> GuestFlowPorts {
            self.process()
                .owner
                .flow_ports()
                .expect("boot step 9's converge_shared bound the owner's listeners")
        }

        /// The process dies (the model's `Crash`): no graceful drain; the
        /// kernel keeps the shared table (`SimMtlsIntercept`'s crash script,
        /// paired with R5-30) and the seam's node-global objects; every
        /// process-scoped object — the owner, its holds, its flows — ends.
        pub async fn crash(&mut self) {
            let io = self.detach_crashed_process().await;
            io.exit_process();
        }

        /// Phase A of abrupt process death for a fixture that owns the whole
        /// process runtime. Abort and join the server-owned tasks, but leave
        /// the seam live until that runtime has been dropped and therefore no
        /// detached owner child can call it.
        pub async fn detach_crashed_process(&mut self) -> Arc<SimGuestFlowIo> {
            let process = self.process.take().expect("a crash needs a live process");
            self.fail_stop_request = None;
            self.intercept.script_crash_retains_table();
            let io = Self::kill_process(process).await;
            self.last_stop = Some(NodeStop::Crash);
            io
        }

        /// The supervisor's fail-stop request has been received; the process
        /// then ends as `serve` does on that request (the table stays).
        pub async fn end_after_fail_stop(&mut self) -> ServeShutdownRequest {
            let (request, io) = self.detach_after_fail_stop().await;
            io.exit_process();
            request
        }

        /// Phase A of a fail-stop process end for a fixture that owns the
        /// whole process runtime. The cached one-shot request is consumed
        /// exactly once; seam exit follows only after that runtime is gone.
        pub async fn detach_after_fail_stop(
            &mut self,
        ) -> (ServeShutdownRequest, Arc<SimGuestFlowIo>) {
            let request = match self.fail_stop_request.take() {
                Some(request) => request,
                None => self.process_mut().server.shutdown_requested().await,
            };
            let process = self.process.take().expect("a fail-stop ends a live process");
            self.intercept.script_crash_retains_table();
            let io = Self::kill_process(process).await;
            self.last_stop = Some(NodeStop::FailStop);
            (request, io)
        }

        /// End the server-owned part of one composed test process. The
        /// process-local owner and dial references go first, then the server
        /// abort/join fence. A fixture that owns the whole process runtime can
        /// now destroy its detached children before releasing the returned
        /// seam; ambient-runtime callers retain their existing immediate-exit
        /// behavior.
        async fn kill_process(process: Process) -> Arc<SimGuestFlowIo> {
            let Process { io, owner, dial, server, dns: _, cid: _ } = process;
            drop(owner);
            drop(dial);
            let _residue = server.kill_for_test().await;
            io
        }

        /// The fail-stop request, if the supervisor has made one, without
        /// waiting.
        pub fn fail_stop_requested(&mut self) -> Option<ServeShutdownRequest> {
            use futures::FutureExt as _;
            if let Some(request) = &self.fail_stop_request {
                return Some(request.clone());
            }
            let request = self.process_mut().server.shutdown_requested().now_or_never();
            self.fail_stop_request.clone_from(&request);
            request
        }

        /// Advance the injected clock by `by` and settle.
        pub async fn advance(&self, by: Duration) {
            self.clock.tick(by);
            self.settle().await;
        }

        /// Resolve when no task of the runtime is ready.
        pub async fn settle(&self) {
            if let Some(process) = &self.process {
                process.io.settled().await;
            } else {
                tokio::task::yield_now().await;
            }
        }

        /// Component `c`'s accept task exits (the model's `Damage(c)`).
        pub async fn damage(&self, component: SharedGuestNetworkComponent) {
            self.process()
                .io
                .fail_listener(listener_of(component), io::ErrorKind::ConnectionAborted);
            self.settle().await;
        }

        /// Release every held post-repair or detecting audit read.
        pub async fn release_audits(&self) {
            while self.process().dns.pending_audits() > 0 {
                self.process().dns.release_audit();
                self.settle().await;
            }
        }

        /// The held repair of `component`, if one is parked: the held
        /// listener's check its recovery's `converge_shared` issued
        /// (`bind_listener` on the held socket).
        pub fn held_repair(
            &self,
            component: SharedGuestNetworkComponent,
        ) -> Option<crate::guest_network::flow_io::sim::SimEffectId> {
            self.process().io.held().into_iter().find_map(|(id, effect)| {
                (effect.kind == SimEffectKind::BindListener
                    && effect.target == SimEffectTarget::Listener(listener_of(component)))
                .then_some(id)
            })
        }

        /// `component`'s parked repair returns `Ok` (`release`: the held
        /// socket still listens, its accept task is re-spawned) or a scripted
        /// `Err` (the held socket is found not listening — the defect the
        /// round-11 "Repair" contract reports as a failed recovery attempt;
        /// nothing is closed or rebound either way).
        pub async fn decide_repair(&self, component: SharedGuestNetworkComponent, ok: bool) {
            let repair = self.held_repair(component).unwrap_or_else(|| {
                panic!("{component:?}'s repair is parked at its held listener's check")
            });
            let seam = &self.process().io;
            if ok {
                seam.release(repair);
            } else {
                seam.fail(repair, SimFailure::Io(io::ErrorKind::InvalidInput));
            }
            self.settle().await;
        }

        /// The model's `LeakHold` (D-UP-3): the composed owner's `quiesce()`,
        /// and the returned hold dropped without `restore`.
        pub async fn leak_hold(&self) {
            let (hold, _quiescence) = self.process().owner.quiesce().await;
            drop(hold);
            self.settle().await;
        }

        /// The model's `ForeignRestore` (D-UP-4, PD-1): a hold issued by a
        /// second `QuiescenceLedger`, restored at the composed owner. Returns
        /// the owner's answer and the issuing ledger (its `leaked()` counts
        /// the hold the owner dropped unredeemed).
        pub async fn restore_foreign_hold(
            &self,
        ) -> (std::result::Result<ForwardingRestore, QuiescenceRestoreError>, QuiescenceLedger)
        {
            let ledger = QuiescenceLedger::new();
            let hold = ledger.issue();
            let restored = self.process().owner.restore(hold).await;
            self.settle().await;
            (restored, ledger)
        }

        /// The `Forwarder` component's context fails `Io` (EVMOD-CTL): the
        /// next audit reports the component damaged.
        pub async fn damage_forwarder(&self) {
            self.process().io.script_failure(
                SimEffectKind::Inventory,
                SimFailure::Forwarder(SimForwarderFailure::ContextIo(ForwarderOp::Inventory)),
            );
            self.settle().await;
        }

        /// The `Forwarder` repair's parked reopen (`open_forwarder`), if any.
        pub fn held_forwarder_reopen(&self) -> Option<SimEffectId> {
            self.process().io.held_of(SimEffectKind::OpenForwarder)
        }

        /// The kinds of every effect the seam issued, in issue order.
        pub fn issued_kinds(&self) -> Vec<SimEffectKind> {
            self.process().io.issued().into_iter().map(|effect| effect.kind).collect()
        }

        /// The kinds of every effect the last refused boot issued, in order.
        pub fn refused_boot_issued_kinds(&self) -> Vec<SimEffectKind> {
            self.refused_io
                .as_ref()
                .expect("a boot was refused")
                .issued()
                .into_iter()
                .map(|effect| effect.kind)
                .collect()
        }

        /// Release one parked effect and settle.
        pub async fn release(&self, effect: SimEffectId) {
            self.process().io.release(effect);
            self.settle().await;
        }

        /// The owner's QPROJ-3 observation.
        pub async fn owner_observation(&self) -> GuestOwnerObservation {
            self.process().owner.flow_observation().await
        }

        /// The runtime supervisor's QPROJ-6 observation.
        pub fn recovery_observation(
            &self,
        ) -> std::collections::BTreeMap<SharedGuestNetworkComponent, RecoveryEpisode> {
            self.process().server.shared_network_supervisor.recovery_observation()
        }

        /// The QPROJ-7 arbiter read of the shared table.
        pub fn table(&self) -> SimSharedTable {
            self.intercept.shared_table_state()
        }

        /// The number of flows the owner holds (guest-opened, host-opened and
        /// datagram associations), from the QPROJ-3 observation.
        pub async fn flows_held(&self) -> usize {
            self.owner_observation()
                .await
                .cids
                .values()
                .map(|cid| cid.guest_flows.len() + cid.dials.len() + cid.datagrams.len())
                .sum()
        }

        /// `provision` the plan's allocation, then the guest opens its control
        /// session: `ControlOpen` with the plan's launch token on the control
        /// listener, accepted and read.
        pub async fn provision_with_session(&self, plan: &GuestNetworkPlan) {
            self.process().owner.provision(plan).await.expect("provision the allocation");
            let control = GuestConnKey {
                listener: self.flow_ports().control(),
                peer_cid: plan.assignment().cid(),
                peer_port: VsockPort::new(49_000),
            };
            let launch = plan.assignment().launch();
            self.deliver(
                FlowListener::Control,
                control,
                ControlOpen { launch }.encode().to_vec(),
                launch,
            )
            .await;
        }

        /// Make one allocation Active with a live control session (lease,
        /// `provision`, the guest's `ControlOpen`, `activate` → `Activated`),
        /// unless the process already has one.
        pub async fn ensure_active_cid(&mut self) {
            if self.process().cid.is_some() {
                return;
            }
            let alloc =
                AllocationId::new(&format!("nd295-qhold-{}", self.boots)).expect("allocation id");
            let plan = self
                .pool
                .assign(
                    alloc,
                    &[ListenPort::new(LISTEN_PORT).expect("listen port")],
                    self.flow_ports(),
                )
                .expect("the rig's pool assigns a lease");
            self.provision_with_session(&plan).await;
            let activation = Arc::clone(&self.process().owner)
                .activate(&plan)
                .await
                .expect("activate the rig's allocation");
            assert_eq!(
                activation,
                ForwardingActivation::Activated,
                "the rig activates with a live session and no hold"
            );
            self.process_mut().cid =
                Some(ActiveCid { plan, next_peer_port: 50_000, next_flow_seq: 1 });
        }

        /// A guest-opened TCP flow to [`FLOW_DESTINATION`] is admitted, its
        /// request read, installed, connected and paired (the model's
        /// `OpenFlow`). Returns the connection key.
        pub async fn open_guest_flow(&mut self) -> GuestConnKey {
            self.ensure_active_cid().await;
            let (cid, launch, peer_port, seq) = {
                let active = self.process_mut().cid.as_mut().expect("an Active allocation");
                let peer_port = active.next_peer_port;
                let seq = active.next_flow_seq;
                active.next_peer_port += 1;
                active.next_flow_seq += 1;
                (active.plan.assignment().cid(), active.plan.assignment().launch(), peer_port, seq)
            };
            self.process().io.set_route_type(*FLOW_DESTINATION.ip(), FibRouteType::Unicast);
            let key = GuestConnKey {
                listener: self.flow_ports().stream(),
                peer_cid: cid,
                peer_port: VsockPort::new(peer_port),
            };
            let request = FlowRequest::TcpConnect {
                flow: FlowId::new(FlowOrigin::Guest, seq).expect("guest flow id"),
                destination: FLOW_DESTINATION,
                launch,
            };
            self.deliver(FlowListener::Stream, key, request.encode().to_vec(), launch).await;
            key
        }

        /// A guest connection on `listener` with `record` buffered; the seam
        /// delivers its accept and its bytes at once (auto-delivery).
        pub async fn deliver(
            &self,
            listener: FlowListener,
            key: GuestConnKey,
            record: Vec<u8>,
            physical_launch: LaunchToken,
        ) {
            self.process().io.queue_connection(listener, key, Some(record), Some(physical_launch));
            self.settle().await;
        }

        /// The Active allocation's CID and launch, if the rig opened one.
        pub fn active_cid(&self) -> Option<(GuestCid, LaunchToken)> {
            self.process()
                .cid
                .as_ref()
                .map(|active| (active.plan.assignment().cid(), active.plan.assignment().launch()))
        }

        /// The Active allocation's plan, if the rig opened one.
        pub fn active_plan(&self) -> Option<&GuestNetworkPlan> {
            self.process().cid.as_ref().map(|active| &active.plan)
        }
    }

    /// One `serve` process composed over the test-local [`TestSharedOwner`]
    /// at the owner port (FHOLD-CHECK): the production supervisor, boot and
    /// EXEC gate, with an owner whose `restore` answers `ForeignHold` when
    /// scripted. No seam is added: the owner is the existing driven port of
    /// `run_server_with_obs_and_drivers`, which takes the node's
    /// `BootAdapters` (the stores and the CA included, STORES-IN).
    pub struct TestOwnerProcess {
        pub owner: Arc<TestSharedOwner>,
        pub server: ServerHandle,
        pub clock: Arc<SimClock>,
        _dirs: TempDir,
    }

    impl TestOwnerProcess {
        /// Boot over `owner`; panics on a boot refusal (the cell's
        /// precondition).
        pub async fn boot(owner: Arc<TestSharedOwner>) -> Self {
            let mut node = ComposedNode::fresh();
            node.boots += 1;
            let clock = Arc::clone(&node.clock);
            let config = node.config();
            let boot = node
                .boot_adapters(&config, Arc::new(TestGuestDnsFactory::default()))
                .expect("the node's stores open over a fresh data directory");
            let mut registry = DriverRegistry::new();
            registry.insert(Arc::new(SimDriver::new(DriverType::Vm)));
            let server = run_server_with_obs_and_drivers(
                config,
                boot,
                Arc::new(registry),
                Arc::clone(&owner) as Arc<dyn crate::guest_network::SharedGuestNetworkOwner>,
                GuestNetworkExecWiring::new(clock.clone()),
            )
            .await
            .expect("the composition over the test-local owner boots");
            let dirs = std::mem::replace(&mut node.dirs, TempDir::new().expect("tempdir"));
            Self { owner, server, clock, _dirs: dirs }
        }

        /// Advance the injected clock by `by` and let every ready task run.
        pub async fn advance(&self, by: Duration) {
            self.clock.tick(by);
            for _ in 0..64 {
                tokio::task::yield_now().await;
            }
        }

        /// The supervisor's fail-stop request, if made, without waiting.
        pub fn fail_stop_requested(&mut self) -> Option<ServeShutdownRequest> {
            use futures::FutureExt as _;
            self.server.shutdown_requested().now_or_never()
        }
    }

    fn node_id() -> NodeId {
        NodeId::new("nd295-qhold-node").expect("node id")
    }
}
