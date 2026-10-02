//! Crate-private test-local implementations of the three ports this crate
//! declares — `SharedGuestNetworkOwner` (with its inherited
//! `GuestNetworkProvisioner`), `GuestDns`, and `GuestDnsFactory` — for this
//! crate's source-local tests (GH #295, DISTILL gap B-4).
//!
//! `overdrive-sim` depends on `overdrive-control-plane`, which dev-depends on
//! `overdrive-sim`, so in a source-local test build the sim types implement the
//! traits of a second compiled copy of this crate and do not coerce to
//! `Arc<dyn crate::…>`. These doubles model the parts of the pinned
//! `SimSharedGuestNetworkOwner` / `SimGuestDnsFactory` / `SimGuestDns`
//! contracts the source-local cells exercise. Their shape is test support
//! (`distill/test-scenarios.md` § *Test-local control-plane ports*), not
//! production API.
//!

#![allow(
    dead_code,
    clippy::significant_drop_tightening,
    reason = "test support consumed by the source-local S-ND295-05E, 19, 29A, 30A, and 32 bodies; \
              each port call records and decides under one state lock, so the journal order is \
              the decision order"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use overdrive_core::guest_network::SharedGuestNetworkComponent;
use overdrive_core::guest_network::{GuestNetworkExecGate, GuestNetworkExecWiring};
use overdrive_core::id::AllocationId;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
use overdrive_sim::adapters::{SimCgroupFs, SimEntry};
use overdrive_sim::adapters::{SimIdentityRead, SimMtlsIntercept, SimMtlsResolve};
use overdrive_worker::mtls_intercept_port::MtlsIntercept;
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;
use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::dns_responder::responder::{DnsResponderError, Result as DnsResult};
use crate::dns_responder::{GuestDns, GuestDnsDeps, GuestDnsFactory};
use crate::guest_network::{
    GuestLinkKind, GuestNetworkError, GuestNetworkFact, GuestNetworkOperation, GuestNetworkPlan,
    GuestNetworkProvisioner, Result, SharedGuestNetworkAudit, SharedGuestNetworkAuditError,
    SharedGuestNetworkOwner, TapActivation, TapQuiescence,
};

// ---------------------------------------------------------------------------
// TestSharedOwner
// ---------------------------------------------------------------------------

/// Standing outcome of every subsequent `quiesce_managed_taps` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestQuiesceScript {
    /// `Ok(TapQuiescence)` naming these allocations unconfirmed (those not yet
    /// condemned); empty is full quiescence. The `Default`.
    Unconfirmed(BTreeSet<AllocationId>),
    /// `Err(GuestNetworkError::Io { operation: TapSetDown, .. })`.
    Fail,
    /// A future that never resolves.
    Hang,
    /// The call waits `after` on the owner's clock, then resolves as
    /// `Unconfirmed(unconfirmed)` would: a result that arrives late. Needs
    /// the clock given to [`TestSharedOwner::with_cgroup_snapshots`].
    Late { unconfirmed: BTreeSet<AllocationId>, after: Duration },
}

impl Default for TestQuiesceScript {
    fn default() -> Self {
        Self::Unconfirmed(BTreeSet::new())
    }
}

/// Standing behaviour of every subsequent `audit_shared` call, before any
/// node-level slot or damage is consulted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TestAuditMode {
    /// Consult the node-level slots, then the damage set.
    #[default]
    Normal,
    /// A future that never resolves.
    Hang,
    /// The call panics.
    Panic,
}

/// Outcome of one non-scripted-result port call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestCallOutcome {
    Ok,
    Failed,
}

/// Outcome of one `activate` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestActivateOutcome {
    /// `Ok(TapActivation::Raised)`.
    Raised,
    /// `Ok(TapActivation::QuiescenceLatched)`; nothing mutated.
    Latched,
    /// The source-less `PostconditionMismatch` for a condemned allocation.
    Refused,
}

/// Outcome of one `audit_shared` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestAuditOutcome {
    /// `Ok` with no damaged allocation.
    Healthy,
    /// `Err` naming the first failing node-level component.
    NodeFailed(SharedGuestNetworkComponent),
    /// `Ok` naming these newly condemned allocations.
    Damaged(BTreeSet<AllocationId>),
    /// The call never resolved.
    Hung,
    /// The call panicked.
    Panicked,
}

/// Outcome of one `quiesce_managed_taps` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestQuiesceOutcome {
    /// `Ok` naming these newly condemned allocations unconfirmed.
    Unconfirmed(BTreeSet<AllocationId>),
    /// `Err`.
    Failed,
    /// The call never resolved.
    Hung,
    /// The call began a scripted late result; whether and when that result
    /// was delivered is not journaled.
    Late,
}

/// One port call, its allocation where it has one, and its outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestOwnerCall {
    Provision { alloc: AllocationId, outcome: TestCallOutcome },
    Activate { alloc: AllocationId, outcome: TestActivateOutcome },
    Teardown { alloc: AllocationId, outcome: TestCallOutcome },
    ProbeStartup,
    SweepStale,
    ConvergeShared(TestCallOutcome),
    AuditShared(TestAuditOutcome),
    Quiesce(TestQuiesceOutcome),
    Restore(TestCallOutcome),
}

/// One journal entry: the call, plus — when the owner was built over a
/// `SimCgroupFs` clone — the cgroup snapshot taken as the call began (the E12
/// ordering observation point).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestJournalEntry {
    pub call: TestOwnerCall,
    pub cgroups: Option<BTreeMap<PathBuf, (SimEntry, Vec<u8>)>>,
}

/// The node-level components in canonical audit order.
const AUDIT_COMPONENTS: [SharedGuestNetworkComponent; 12] = [
    SharedGuestNetworkComponent::Bridge,
    SharedGuestNetworkComponent::LegF,
    SharedGuestNetworkComponent::LegC,
    SharedGuestNetworkComponent::Dns,
    SharedGuestNetworkComponent::TcxLink,
    SharedGuestNetworkComponent::EndpointMap,
    SharedGuestNetworkComponent::CounterMap,
    SharedGuestNetworkComponent::BpffsPin,
    SharedGuestNetworkComponent::BridgeGuard,
    SharedGuestNetworkComponent::IpRules,
    SharedGuestNetworkComponent::IpSets,
    SharedGuestNetworkComponent::Supervisor,
];

#[derive(Debug, Default)]
struct TestOwnerState {
    /// Set by every `quiesce_managed_taps`; cleared only by a successful
    /// `restore_quiesced_taps`.
    latched: bool,
    /// Every allocation a returned quiescence or audit result has named.
    condemned: BTreeSet<AllocationId>,
    /// Allocations whose TAP this double holds raised (`Active`): an
    /// activation that reported `Raised`, or a restore that raised it.
    active: BTreeSet<AllocationId>,
    /// Allocations a returned quiescence set down (`QuiescedActive`).
    quiesced: BTreeSet<AllocationId>,
    /// A failing restore raises this many quiesced allocations, in
    /// `AllocationId` order, before it fails.
    restore_raises_before_failure: usize,
    quiesce: TestQuiesceScript,
    audit_mode: TestAuditMode,
    damage: BTreeSet<AllocationId>,
    audit_failures: Vec<SharedGuestNetworkComponent>,
    journal: Vec<TestJournalEntry>,
}

impl TestOwnerState {
    /// A returned quiescence: condemn each newly named allocation (no longer
    /// raised), and set every other raised TAP down.
    fn settle_quiescence(
        &mut self,
        named: &BTreeSet<AllocationId>,
    ) -> BTreeMap<AllocationId, GuestNetworkError> {
        let unconfirmed = TestSharedOwner::condemn(self, named, GuestNetworkOperation::TapSetDown);
        let raised = std::mem::take(&mut self.active);
        for alloc in raised {
            if !self.condemned.contains(&alloc) {
                self.quiesced.insert(alloc);
            }
        }
        unconfirmed
    }
}

/// What one `quiesce_managed_taps` call does once it has journaled.
enum QuiesceStep {
    Resolved(Result<TapQuiescence>),
    Hang,
    Late { unconfirmed: BTreeSet<AllocationId>, after: Duration },
}

/// The clock a [`TestQuiesceScript::Late`] result waits on.
struct LateClock(Arc<dyn Clock>);

impl std::fmt::Debug for LateClock {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LateClock")
    }
}

/// Test-local shared guest-network owner modelling the latch, the
/// `Condemned` set, which TAPs it holds raised or quiesced, standing refusal
/// slots, and a per-call journal.
#[derive(Debug, Default)]
pub struct TestSharedOwner {
    provision_refused: AtomicBool,
    teardown_refused: AtomicBool,
    converge_refused: AtomicBool,
    restore_refused: AtomicBool,
    cgroups: Option<SimCgroupFs>,
    late_clock: Option<LateClock>,
    state: Mutex<TestOwnerState>,
}

impl TestSharedOwner {
    /// An owner with every slot disarmed, full quiescence, an empty damage set,
    /// and no cgroup snapshots in its journal.
    pub fn new() -> Self {
        Self::default()
    }

    /// As [`Self::new`], but every journal entry carries `fs.snapshot()`
    /// taken as the call began, and a [`TestQuiesceScript::Late`] result
    /// waits on `clock` (the supervisor's injected clock).
    pub fn with_cgroup_snapshots(fs: SimCgroupFs, clock: Arc<dyn Clock>) -> Self {
        Self { cgroups: Some(fs), late_clock: Some(LateClock(clock)), ..Self::default() }
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
    /// Arm or disarm the standing restore refusal; an armed restore raises
    /// nothing before it fails.
    pub fn script_restore_failure(&self, armed: bool) {
        self.state.lock().restore_raises_before_failure = 0;
        self.restore_refused.store(armed, Ordering::SeqCst);
    }
    /// Arm a standing restore refusal that fails part-way: each failing
    /// restore first raises up to `raised` quiesced allocations, in
    /// `AllocationId` order, and keeps the latch.
    pub fn script_restore_failure_after(&self, raised: usize) {
        self.state.lock().restore_raises_before_failure = raised;
        self.restore_refused.store(true, Ordering::SeqCst);
    }
    /// Arm or disarm the standing node-level audit slot of `component`.
    pub fn script_component_audit_failure(
        &self,
        component: SharedGuestNetworkComponent,
        armed: bool,
    ) {
        let mut state = self.state.lock();
        state.audit_failures.retain(|armed_component| *armed_component != component);
        if armed {
            state.audit_failures.push(component);
        }
    }
    /// Set the standing damage set the audit reports when no node-level slot
    /// fires.
    pub fn script_audit_damage(&self, damaged: BTreeSet<AllocationId>) {
        self.state.lock().damage = damaged;
    }
    /// Set the standing audit behaviour (normal, hang, or panic).
    pub fn script_audit_mode(&self, mode: TestAuditMode) {
        self.state.lock().audit_mode = mode;
    }
    /// Set the standing quiescence outcome.
    pub fn script_quiesce(&self, script: TestQuiesceScript) {
        self.state.lock().quiesce = script;
    }

    /// Every port call, in call order.
    pub fn journal(&self) -> Vec<TestJournalEntry> {
        self.state.lock().journal.clone()
    }
    /// The quiescence latch `activate` consults.
    pub fn latched(&self) -> bool {
        self.state.lock().latched
    }
    /// Every allocation a returned quiescence or audit result has named.
    pub fn condemned(&self) -> BTreeSet<AllocationId> {
        self.state.lock().condemned.clone()
    }
    /// The allocations whose TAP this double holds raised.
    pub fn active(&self) -> BTreeSet<AllocationId> {
        self.state.lock().active.clone()
    }

    fn record(&self, state: &mut TestOwnerState, call: TestOwnerCall) {
        let cgroups = self.cgroups.as_ref().map(SimCgroupFs::snapshot);
        state.journal.push(TestJournalEntry { call, cgroups });
    }

    fn outcome(armed: &AtomicBool) -> TestCallOutcome {
        if armed.load(Ordering::SeqCst) { TestCallOutcome::Failed } else { TestCallOutcome::Ok }
    }

    fn refusal(operation: GuestNetworkOperation) -> GuestNetworkError {
        GuestNetworkError::Io {
            operation,
            source: std::io::Error::other("scripted test owner refusal"),
        }
    }

    fn result(outcome: TestCallOutcome, operation: GuestNetworkOperation) -> Result<()> {
        match outcome {
            TestCallOutcome::Ok => Ok(()),
            TestCallOutcome::Failed => Err(Self::refusal(operation)),
        }
    }

    /// Condemn each not-yet-condemned allocation of `named`; the returned map
    /// names exactly those, each with a fresh error for `operation`. A
    /// condemned allocation leaves the raised and quiesced sets, so no later
    /// restore raises it.
    fn condemn(
        state: &mut TestOwnerState,
        named: &BTreeSet<AllocationId>,
        operation: GuestNetworkOperation,
    ) -> BTreeMap<AllocationId, GuestNetworkError> {
        let newly: BTreeMap<AllocationId, GuestNetworkError> = named
            .iter()
            .filter(|alloc| state.condemned.insert((*alloc).clone()))
            .map(|alloc| (alloc.clone(), Self::refusal(operation)))
            .collect();
        for alloc in newly.keys() {
            state.active.remove(alloc);
            state.quiesced.remove(alloc);
        }
        newly
    }
}

#[async_trait::async_trait]
impl GuestNetworkProvisioner for TestSharedOwner {
    async fn provision(&self, plan: &GuestNetworkPlan) -> Result<()> {
        let outcome = Self::outcome(&self.provision_refused);
        let mut state = self.state.lock();
        self.record(&mut state, TestOwnerCall::Provision { alloc: plan.alloc().clone(), outcome });
        drop(state);
        Self::result(outcome, GuestNetworkOperation::TapCreate)
    }

    async fn activate(&self, plan: &GuestNetworkPlan) -> Result<TapActivation> {
        let mut state = self.state.lock();
        let outcome = if state.latched {
            TestActivateOutcome::Latched
        } else if state.condemned.contains(plan.alloc()) {
            TestActivateOutcome::Refused
        } else {
            TestActivateOutcome::Raised
        };
        if outcome == TestActivateOutcome::Raised {
            state.active.insert(plan.alloc().clone());
        }
        self.record(&mut state, TestOwnerCall::Activate { alloc: plan.alloc().clone(), outcome });
        drop(state);
        match outcome {
            TestActivateOutcome::Raised => Ok(TapActivation::Raised),
            TestActivateOutcome::Latched => Ok(TapActivation::QuiescenceLatched),
            TestActivateOutcome::Refused => Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::TapObserve,
                expected: GuestNetworkFact::Tap {
                    name: plan.assignment().tap.clone(),
                    ifindex: None,
                    link_kind: GuestLinkKind::Tap,
                    persistent: true,
                    up: false,
                    owner_uid: Some(0),
                },
                observed: None,
            }),
        }
    }

    async fn teardown(&self, plan: &GuestNetworkPlan) -> Result<()> {
        let outcome = Self::outcome(&self.teardown_refused);
        let mut state = self.state.lock();
        self.record(&mut state, TestOwnerCall::Teardown { alloc: plan.alloc().clone(), outcome });
        drop(state);
        Self::result(outcome, GuestNetworkOperation::TapDelete)
    }
}

#[async_trait::async_trait]
impl SharedGuestNetworkOwner for TestSharedOwner {
    async fn probe_startup(&self) -> Result<()> {
        let mut state = self.state.lock();
        self.record(&mut state, TestOwnerCall::ProbeStartup);
        Ok(())
    }

    async fn sweep_stale(&self) -> Result<()> {
        let mut state = self.state.lock();
        self.record(&mut state, TestOwnerCall::SweepStale);
        Ok(())
    }

    async fn converge_shared(&self) -> Result<()> {
        let outcome = Self::outcome(&self.converge_refused);
        let mut state = self.state.lock();
        self.record(&mut state, TestOwnerCall::ConvergeShared(outcome));
        drop(state);
        Self::result(outcome, GuestNetworkOperation::BridgeConverge)
    }

    #[allow(clippy::panic, reason = "a scripted audit panic is this double's contract")]
    async fn audit_shared(
        &self,
    ) -> std::result::Result<SharedGuestNetworkAudit, SharedGuestNetworkAuditError> {
        let result = {
            let mut state = self.state.lock();
            let mode = state.audit_mode;
            match mode {
                TestAuditMode::Hang => {
                    self.record(&mut state, TestOwnerCall::AuditShared(TestAuditOutcome::Hung));
                    None
                }
                TestAuditMode::Panic => {
                    self.record(&mut state, TestOwnerCall::AuditShared(TestAuditOutcome::Panicked));
                    drop(state);
                    panic!("scripted test owner audit panic");
                }
                TestAuditMode::Normal => {
                    let failed = AUDIT_COMPONENTS
                        .into_iter()
                        .find(|component| state.audit_failures.contains(component));
                    if let Some(component) = failed {
                        self.record(
                            &mut state,
                            TestOwnerCall::AuditShared(TestAuditOutcome::NodeFailed(component)),
                        );
                        Some(Err(SharedGuestNetworkAuditError {
                            component,
                            source: GuestNetworkError::PostconditionMismatch {
                                operation: GuestNetworkOperation::SharedAudit,
                                expected: GuestNetworkFact::SharedComponent {
                                    component,
                                    healthy: true,
                                },
                                observed: Some(GuestNetworkFact::SharedComponent {
                                    component,
                                    healthy: false,
                                }),
                            },
                        }))
                    } else {
                        let damage = state.damage.clone();
                        let damaged =
                            Self::condemn(&mut state, &damage, GuestNetworkOperation::TapObserve);
                        let outcome = if damaged.is_empty() {
                            TestAuditOutcome::Healthy
                        } else {
                            TestAuditOutcome::Damaged(damaged.keys().cloned().collect())
                        };
                        self.record(&mut state, TestOwnerCall::AuditShared(outcome));
                        Some(Ok(SharedGuestNetworkAudit { damaged }))
                    }
                }
            }
        };
        match result {
            Some(result) => result,
            None => std::future::pending().await,
        }
    }

    #[allow(clippy::panic, reason = "a Late script on an owner without a clock is fixture misuse")]
    async fn quiesce_managed_taps(&self) -> Result<TapQuiescence> {
        let step = {
            let mut state = self.state.lock();
            state.latched = true;
            match state.quiesce.clone() {
                TestQuiesceScript::Unconfirmed(named) => {
                    let unconfirmed = state.settle_quiescence(&named);
                    let reported = unconfirmed.keys().cloned().collect();
                    self.record(
                        &mut state,
                        TestOwnerCall::Quiesce(TestQuiesceOutcome::Unconfirmed(reported)),
                    );
                    QuiesceStep::Resolved(Ok(TapQuiescence { unconfirmed }))
                }
                TestQuiesceScript::Fail => {
                    self.record(&mut state, TestOwnerCall::Quiesce(TestQuiesceOutcome::Failed));
                    QuiesceStep::Resolved(Err(Self::refusal(GuestNetworkOperation::TapSetDown)))
                }
                TestQuiesceScript::Hang => {
                    self.record(&mut state, TestOwnerCall::Quiesce(TestQuiesceOutcome::Hung));
                    QuiesceStep::Hang
                }
                TestQuiesceScript::Late { unconfirmed, after } => {
                    self.record(&mut state, TestOwnerCall::Quiesce(TestQuiesceOutcome::Late));
                    QuiesceStep::Late { unconfirmed, after }
                }
            }
        };
        match step {
            QuiesceStep::Resolved(result) => result,
            QuiesceStep::Hang => std::future::pending().await,
            QuiesceStep::Late { unconfirmed, after } => {
                let Some(LateClock(clock)) = self.late_clock.as_ref() else {
                    panic!(
                        "a Late quiescence needs the clock given to \
                         TestSharedOwner::with_cgroup_snapshots"
                    );
                };
                clock.sleep(after).await;
                let unconfirmed = self.state.lock().settle_quiescence(&unconfirmed);
                Ok(TapQuiescence { unconfirmed })
            }
        }
    }

    async fn restore_quiesced_taps(&self) -> Result<()> {
        let outcome = Self::outcome(&self.restore_refused);
        let mut state = self.state.lock();
        self.record(&mut state, TestOwnerCall::Restore(outcome));
        let raising = match outcome {
            TestCallOutcome::Ok => state.quiesced.len(),
            TestCallOutcome::Failed => state.restore_raises_before_failure,
        };
        let raised = state.quiesced.iter().take(raising).cloned().collect::<Vec<_>>();
        for alloc in raised {
            state.quiesced.remove(&alloc);
            state.active.insert(alloc);
        }
        if outcome == TestCallOutcome::Ok {
            state.latched = false;
        }
        drop(state);
        Self::result(outcome, GuestNetworkOperation::TapSetUp)
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
    built: Mutex<Vec<Arc<TestGuestDns>>>,
}

impl TestGuestDnsFactory {
    pub fn script_probe_failure(&self, armed: bool) {
        self.probe_refused.store(armed, Ordering::SeqCst);
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
// Self-tests of the test-local shared owner (S-ND295-53)
// ---------------------------------------------------------------------------
//
// These specify the `SharedGuestNetworkOwner` activate/restore contract the
// S-ND295-53 activation-order proof and the source-local supervisor bodies
// depend on, over the double they actually use here: `TestSharedOwner`. The
// sim-side twin (`SimSharedGuestNetworkOwner`) cannot host them because its
// `activate(&GuestNetworkPlan)` needs a `GuestNetworkPlan`, and that type is
// control-plane-private with no cross-crate constructor (FD § "B1 — replace
// obsolete plan vocabulary"); a plan is built here through a test-owned
// `GuestAddressPool`'s crate-private `assign`, as the source-local supervisor
// rig does, so no process-global pool is touched. "A failed restore keeps the
// latch" has no other coverage, so a double that regressed it would pass the
// SUT proof silently — that is exactly what these guard against (a fixture
// must not fail before, or lie to, the SUT).

pub fn mtls_worker() -> Arc<MtlsInterceptWorker> {
    let enforcement: Arc<dyn MtlsEnforcement> = Arc::new(SimMtlsEnforcement::new(
        Arc::new(SimIdentityRead::new(BTreeMap::new(), None)),
        MtlsLimits::default(),
    ));
    let resolve: Arc<dyn MtlsResolve> =
        Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh));
    let intercept: Arc<dyn MtlsIntercept> = Arc::new(SimMtlsIntercept::new());
    Arc::new(MtlsInterceptWorker::new(enforcement, resolve, Arc::new(SimClock::new()), intercept))
}

pub fn shared_guest_network_owner() -> Arc<TestSharedOwner> {
    Arc::new(TestSharedOwner::new())
}

pub fn guest_network_exec_gate() -> Arc<GuestNetworkExecGate> {
    GuestNetworkExecWiring::new(Arc::new(SimClock::new())).gate()
}

pub fn guest_pool() -> Arc<crate::guest_network::GuestAddressPool> {
    Arc::new(crate::guest_network::GuestAddressPool::new(
        ipnet::Ipv4Net::new_assert(std::net::Ipv4Addr::new(100, 95, 0, 0), 16),
        "ovd-gbr0".to_owned(),
        std::net::Ipv4Addr::new(100, 95, 0, 1),
        std::net::Ipv4Addr::new(100, 95, 0, 1),
    ))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use std::net::Ipv4Addr;

    use super::TestSharedOwner;
    use crate::guest_network::{
        GuestAddressPool, GuestNetworkError, GuestNetworkOperation, GuestNetworkProvisioner,
        SharedGuestNetworkOwner, TapActivation,
    };
    use overdrive_core::id::AllocationId;

    fn alloc(name: &str) -> AllocationId {
        AllocationId::new(name).expect("valid allocation id")
    }

    /// A pool owned by one self-test, over the node guest prefix.
    fn pool() -> GuestAddressPool {
        GuestAddressPool::new(
            ipnet::Ipv4Net::new(Ipv4Addr::new(100, 95, 0, 0), 16).expect("node guest prefix"),
            "ovd-gbr0".to_owned(),
            Ipv4Addr::new(100, 95, 0, 1),
            Ipv4Addr::new(100, 95, 0, 1),
        )
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-53 — Activation waits out a recovery and never turns it into a failure
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// `activate` reports `Raised` on a fresh allocation, `QuiescenceLatched`
    /// while a quiescence latch is set (raising nothing), and the source-less
    /// `PostconditionMismatch` for an allocation a returned quiescence result
    /// condemned. Re-homed from `overdrive-sim` because a `GuestNetworkPlan`
    /// is control-plane-private (item 7, DISTILL follow-up).
    #[tokio::test]
    async fn activate_reports_raised_latched_or_condemned() {
        let owner = TestSharedOwner::new();
        let pool = pool();
        let plan_a = pool.assign(alloc("s53-selftest-a")).expect("plan A");
        let plan_b = pool.assign(alloc("s53-selftest-b")).expect("plan B");

        // Fresh, no latch, not condemned → Raised.
        assert_eq!(
            owner.activate(&plan_a).await.expect("a fresh activation raises"),
            TapActivation::Raised
        );

        // A latch makes every activation return QuiescenceLatched, raising nothing.
        owner.quiesce_managed_taps().await.expect("full quiescence latches");
        assert!(owner.latched(), "quiescence sets the latch");
        assert_eq!(
            owner.activate(&plan_a).await.expect("a latched activation is not a failure"),
            TapActivation::QuiescenceLatched
        );

        // Restore clears the latch; activation raises again.
        owner.restore_quiesced_taps().await.expect("restore clears the latch");
        assert!(!owner.latched(), "a successful restore clears the latch");
        assert_eq!(
            owner.activate(&plan_a).await.expect("activation after restore"),
            TapActivation::Raised
        );

        // Condemn plan B through a scripted quiescence, then clear the latch so
        // the condemnation — not the latch — is the reason it is refused.
        owner.script_quiesce(super::TestQuiesceScript::Unconfirmed(
            std::collections::BTreeSet::from([alloc("s53-selftest-b")]),
        ));
        let quiescence = owner.quiesce_managed_taps().await.expect("scripted quiescence");
        assert_eq!(
            quiescence.unconfirmed.keys().cloned().collect::<Vec<_>>(),
            vec![alloc("s53-selftest-b")],
            "the scripted allocation is reported unconfirmed"
        );
        assert!(owner.condemned().contains(&alloc("s53-selftest-b")), "and condemned");
        owner.restore_quiesced_taps().await.expect("restore clears the latch");
        let refusal = owner.activate(&plan_b).await.expect_err("a condemned allocation is refused");
        assert!(
            matches!(
                refusal,
                GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::TapObserve,
                    ..
                }
            ),
            "the refusal is the source-less missing-record mismatch: {refusal:?}"
        );
        // An uncondemned allocation still raises.
        assert_eq!(
            owner.activate(&plan_a).await.expect("an uncondemned allocation still raises"),
            TapActivation::Raised
        );
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-53 — Activation waits out a recovery and never turns it into a failure
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A failed `restore_quiesced_taps` returns `Err` and leaves the
    /// quiescence latch set, so a subsequent activation still returns
    /// `QuiescenceLatched`; only a successful restore clears it. Nothing else
    /// covers the failed-restore-keeps-the-latch path (item 7, DISTILL
    /// follow-up).
    #[tokio::test]
    async fn restore_failure_slot_keeps_the_latch() {
        let owner = TestSharedOwner::new();
        let plan = pool().assign(alloc("s53-selftest-restore")).expect("plan");

        owner.quiesce_managed_taps().await.expect("full quiescence latches");
        assert!(owner.latched(), "quiescence sets the latch");

        owner.script_restore_failure(true);
        let failed = owner.restore_quiesced_taps().await;
        assert!(
            matches!(
                failed,
                Err(GuestNetworkError::Io { operation: GuestNetworkOperation::TapSetUp, .. })
            ),
            "an armed restore fails with the typed set-up error: {failed:?}"
        );
        assert!(owner.latched(), "a failed restore keeps the latch");
        assert_eq!(
            owner.activate(&plan).await.expect("still latched after the failed restore"),
            TapActivation::QuiescenceLatched
        );

        owner.script_restore_failure(false);
        owner.restore_quiesced_taps().await.expect("the retry restores and clears the latch");
        assert!(!owner.latched(), "the successful retry clears the latch");
        assert_eq!(
            owner.activate(&plan).await.expect("activation after the successful restore"),
            TapActivation::Raised
        );
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-29A — Every shared-network component follows one bounded recovery contract
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// The part-way restore the E11 re-quiescence cell scripts: a failing
    /// restore raises the scripted number of quiesced allocations, in
    /// `AllocationId` order, and keeps the latch; a repeat quiescence sets
    /// them down again; a successful restore raises every quiesced
    /// allocation and clears the latch; a condemned allocation is never
    /// raised.
    #[tokio::test]
    async fn a_part_way_restore_raises_some_and_a_repeat_quiescence_sets_them_down() {
        let owner = TestSharedOwner::new();
        let pool = pool();
        let names = ["s29a-selftest-a", "s29a-selftest-b", "s29a-selftest-c"];
        for name in names {
            let plan = pool.assign(alloc(name)).expect("plan");
            assert_eq!(
                owner.activate(&plan).await.expect("a fresh activation raises"),
                TapActivation::Raised
            );
        }
        let all = names.into_iter().map(alloc).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(owner.active(), all, "every raised TAP is held raised");

        owner.quiesce_managed_taps().await.expect("full quiescence");
        assert!(owner.active().is_empty(), "quiescence sets every raised TAP down");

        owner.script_restore_failure_after(1);
        owner.restore_quiesced_taps().await.expect_err("the restore fails part-way");
        assert_eq!(
            owner.active(),
            std::collections::BTreeSet::from([alloc("s29a-selftest-a")]),
            "the failing restore raised the first quiesced allocation"
        );
        assert!(owner.latched(), "a failed restore keeps the latch");

        owner.quiesce_managed_taps().await.expect("a repeat quiescence");
        assert!(owner.active().is_empty(), "the repeat sets the raised TAP down again");
        assert!(owner.latched());

        owner.script_restore_failure(false);
        owner.restore_quiesced_taps().await.expect("a clean restore");
        assert_eq!(owner.active(), all, "a clean restore raises every quiesced allocation");
        assert!(!owner.latched(), "and clears the latch");

        owner.script_quiesce(super::TestQuiesceScript::Unconfirmed(
            std::collections::BTreeSet::from([alloc("s29a-selftest-b")]),
        ));
        owner.quiesce_managed_taps().await.expect("a quiescence naming one allocation");
        owner.restore_quiesced_taps().await.expect("a clean restore");
        assert_eq!(
            owner.active(),
            std::collections::BTreeSet::from([alloc("s29a-selftest-a"), alloc("s29a-selftest-c")]),
            "a condemned allocation is never raised again"
        );
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-32 — Every supervisor wait is bounded and every abnormal exit closes new commands visibly
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// The late quiescence the S-ND295-32 late-result cell scripts: the call
    /// sets the latch and journals `Late` at once, stays pending until its
    /// delay has elapsed on the owner's clock, and only then condemns and
    /// reports the scripted allocations.
    #[tokio::test]
    async fn a_late_quiescence_resolves_only_after_its_delay_on_the_owner_clock() {
        use futures::FutureExt as _;
        use overdrive_sim::adapters::clock::SimClock;
        use std::sync::Arc;
        use std::time::Duration;

        let clock = Arc::new(SimClock::new());
        let owner = TestSharedOwner::with_cgroup_snapshots(
            overdrive_sim::adapters::SimCgroupFs::new(),
            Arc::clone(&clock) as Arc<dyn overdrive_core::traits::clock::Clock>,
        );
        let late = std::collections::BTreeSet::from([alloc("s32-selftest-late")]);
        owner.script_quiesce(super::TestQuiesceScript::Late {
            unconfirmed: late.clone(),
            after: Duration::from_secs(2),
        });

        let mut call = owner.quiesce_managed_taps();
        assert!((&mut call).now_or_never().is_none(), "the call is pending at once");
        assert!(owner.latched(), "the latch is set as the call begins");
        assert_eq!(
            owner.journal().into_iter().map(|entry| entry.call).collect::<Vec<_>>(),
            [super::TestOwnerCall::Quiesce(super::TestQuiesceOutcome::Late)]
        );
        clock.tick(Duration::from_secs(1));
        assert!((&mut call).now_or_never().is_none(), "still pending before its delay");
        assert!(owner.condemned().is_empty(), "nothing is condemned before the result");
        clock.tick(Duration::from_secs(1));
        let quiescence = (&mut call)
            .now_or_never()
            .expect("the result arrives once its delay has elapsed")
            .expect("the late result is Ok");
        assert_eq!(
            quiescence.unconfirmed.keys().cloned().collect::<std::collections::BTreeSet<_>>(),
            late
        );
        assert_eq!(owner.condemned(), late);
    }
}
