//! Socket-free simulation adapter for the GH #295 shared guest-network owner.
//!
//! Scripts what a consumer of the owner's ports (the action shim, the
//! composition root) branches on: the three `ForwardingActivation` outcomes,
//! standing refusal slots for every non-audit operation, one-shot and standing
//! audit damage, the `OwnerCgroupPlacement` (or any other) startup refusal,
//! and records every port call in order.
//!
//! **Quiescence (QHOLD, D-UP-4, PD-2).** The adapter owns its own
//! `QuiescenceLedger` (public in `overdrive-control-plane`, not `Clone`; one
//! per owner) and a hold count: `quiesce` issues a hold from that ledger and
//! raises the count; `restore` redeems it at the ledger and lowers the count
//! (`Reopened` at zero, else `StillQuiesced`); a hold another ledger issued is
//! refused `Err(QuiescenceRestoreError::ForeignHold)` and changes nothing (it
//! is dropped, counted leaked at its issuing ledger); a hold of this ledger
//! dropped unredeemed is counted by `leaked_quiescence_holds`. While the count
//! is above zero every activation answers `QuiescenceLatched`. It aborts no
//! flow (it holds none).

#![allow(
    clippy::result_large_err,
    reason = "the sim implements the exact accepted control-plane GuestNetworkError contract"
)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::net::SocketAddrV4;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use overdrive_control_plane::guest_network::{
    ForwardingActivation, ForwardingQuiescence, ForwardingRestore, GuestNetworkError,
    GuestNetworkFact, GuestNetworkOperation, GuestNetworkPlan, GuestNetworkProvisioner,
    QuiescenceHold, QuiescenceLedger, QuiescenceRestoreError, Result, SharedGuestNetworkAudit,
    SharedGuestNetworkAuditError, SharedGuestNetworkOwner,
};
use overdrive_core::guest_network::{GuestNetworkExecWiring, SharedGuestNetworkComponent};
use overdrive_core::guest_transport::dial::{
    DialPurpose, GuestDialError, GuestStream, GuestStreamDial,
};
use overdrive_core::guest_transport::{
    GUEST_FLOW_CONTROL_PORT, GUEST_FLOW_STREAM_PORT, GuestFlowPorts,
};
use overdrive_core::id::AllocationId;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::vm_host_state::{VmHostObservation, VmHostState};
use parking_lot::Mutex;

use super::vm_host_state::SimVmHostState;

/// Host-state snapshot observed at one real Sim shared-owner sweep call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimSharedGuestNetworkSweepCall {
    pub call_index: usize,
    pub host: VmHostObservation,
}

/// One `dial` the adapter answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimSharedGuestNetworkDial {
    pub target: SocketAddrV4,
    pub purpose: DialPurpose,
}

#[derive(Debug, Default)]
struct Trace {
    calls: Vec<GuestNetworkOperation>,
    sweep_calls: Vec<SimSharedGuestNetworkSweepCall>,
    dials: Vec<SimSharedGuestNetworkDial>,
    activations: Vec<ForwardingActivation>,
}

/// The adapter's counted quiescence over its own ledger (module doc).
struct SimQuiescence {
    ledger: QuiescenceLedger,
    holds: AtomicU32,
}

impl Default for SimQuiescence {
    fn default() -> Self {
        Self { ledger: QuiescenceLedger::new(), holds: AtomicU32::new(0) }
    }
}

impl std::fmt::Debug for SimQuiescence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimQuiescence")
            .field("holds", &self.holds.load(Ordering::SeqCst))
            .field("leaked", &self.ledger.leaked())
            .finish()
    }
}

/// Sim owner with one standing failure slot per non-audit operation, a
/// scripted sequence of activation outcomes, audit damage, and a counted
/// quiescence over its own ledger.
#[derive(Debug, Default)]
pub struct SimSharedGuestNetworkOwner {
    provision: AtomicBool,
    teardown: AtomicBool,
    probe: AtomicBool,
    sweep: AtomicBool,
    converge: AtomicBool,
    /// Set by the first `Ok` `converge_shared` (PORTS-CONFIG: `flow_ports`
    /// is `Some` from then on).
    converged: AtomicBool,
    audit_components: [AtomicBool; 9],
    activations: Mutex<VecDeque<ForwardingActivation>>,
    standing_activation: Mutex<Option<ForwardingActivation>>,
    audit_damage: Mutex<BTreeSet<AllocationId>>,
    condemned: Mutex<BTreeSet<AllocationId>>,
    probe_error: Mutex<Option<GuestNetworkError>>,
    audit_error: Mutex<Option<(SharedGuestNetworkComponent, GuestNetworkError)>>,
    sweep_host_state: Option<SimVmHostState>,
    quiescence: SimQuiescence,
    trace: Mutex<Trace>,
}

impl SimSharedGuestNetworkOwner {
    /// Bind the existing Arc-backed simulated host state to real sweep-call observation.
    #[must_use]
    pub fn with_sweep_host_state(host: SimVmHostState) -> Self {
        Self { sweep_host_state: Some(host), ..Self::default() }
    }

    pub fn script_provision_failure(&self, armed: bool) {
        self.provision.store(armed, Ordering::SeqCst);
    }
    /// QHOLD: the outstanding hold count (leaked holds included).
    pub fn quiescence_holds(&self) -> u32 {
        self.quiescence.holds.load(Ordering::SeqCst)
    }
    /// QHOLD: holds of this adapter's ledger dropped unredeemed.
    pub fn leaked_quiescence_holds(&self) -> u32 {
        self.quiescence.ledger.leaked()
    }
    pub fn script_teardown_failure(&self, armed: bool) {
        self.teardown.store(armed, Ordering::SeqCst);
    }
    pub fn script_probe_failure(&self, armed: bool) {
        self.probe.store(armed, Ordering::SeqCst);
    }
    /// Script the next startup-probe result with the exact accepted typed
    /// error — `OwnerCgroupPlacement`, `FdBudget`, a guest-prefix overlap, or
    /// a primary+cleanup aggregate.
    pub fn script_next_probe_error(&self, error: GuestNetworkError) {
        *self.probe_error.lock() = Some(error);
    }
    pub fn script_sweep_failure(&self, armed: bool) {
        self.sweep.store(armed, Ordering::SeqCst);
    }
    pub fn script_converge_failure(&self, armed: bool) {
        self.converge.store(armed, Ordering::SeqCst);
    }
    pub fn script_audit_failure(&self, armed: bool) {
        self.script_component_audit_failure(SharedGuestNetworkComponent::Forwarder, armed);
    }
    pub fn script_component_audit_failure(
        &self,
        component: SharedGuestNetworkComponent,
        armed: bool,
    ) {
        self.audit_components[Self::component_index(component)].store(armed, Ordering::SeqCst);
    }
    pub fn script_next_audit_error(
        &self,
        component: SharedGuestNetworkComponent,
        source: GuestNetworkError,
    ) {
        *self.audit_error.lock() = Some((component, source));
    }
    /// The next activations return these outcomes, in order; afterwards
    /// `Activated`. `QuiescenceLatched` and `ControlSessionPending` are the
    /// two waiting outcomes the shim retries under the EXEC claim.
    pub fn script_activations(&self, outcomes: impl IntoIterator<Item = ForwardingActivation>) {
        self.activations.lock().extend(outcomes);
    }
    /// While `Some(outcome)`, every activation returns `outcome` (for
    /// example `QuiescenceLatched` for as long as the consumer under test
    /// should see forwarding quiesced); `None` returns to the scripted
    /// sequence, then `Activated`.
    pub fn script_standing_activation(&self, outcome: Option<ForwardingActivation>) {
        *self.standing_activation.lock() = outcome;
    }
    /// Every activation outcome returned, in call order.
    #[must_use]
    pub fn activation_outcomes(&self) -> Vec<ForwardingActivation> {
        self.trace.lock().activations.clone()
    }
    /// Set the standing damage set `audit_shared` reports when no node-level
    /// audit slot fires.
    pub fn script_audit_damage(&self, damaged: BTreeSet<AllocationId>) {
        *self.audit_damage.lock() = damaged;
    }

    /// Ordered production-port calls observed by this simulation adapter.
    #[must_use]
    pub fn calls(&self) -> Vec<GuestNetworkOperation> {
        self.trace.lock().calls.clone()
    }

    /// Ordered host snapshots taken by the actual `sweep_stale` port call.
    #[must_use]
    pub fn sweep_calls(&self) -> Vec<SimSharedGuestNetworkSweepCall> {
        self.trace.lock().sweep_calls.clone()
    }

    /// Every dial answered, in order.
    #[must_use]
    pub fn dials(&self) -> Vec<SimSharedGuestNetworkDial> {
        self.trace.lock().dials.clone()
    }

    fn record(&self, operation: GuestNetworkOperation) {
        self.trace.lock().calls.push(operation);
    }

    fn result(armed: &AtomicBool, operation: GuestNetworkOperation) -> Result<()> {
        if armed.load(Ordering::SeqCst) { Err(Self::refusal(operation)) } else { Ok(()) }
    }

    /// The fixed typed error every scripted non-audit refusal returns, with a
    /// fresh source per call.
    fn refusal(operation: GuestNetworkOperation) -> GuestNetworkError {
        GuestNetworkError::Io {
            operation,
            source: std::io::Error::other("scripted sim owner refusal"),
        }
    }

    /// Condemn every not-yet-condemned allocation of `named`, mapping each to
    /// a fresh typed error. Already-condemned allocations are never named again.
    fn condemn(&self, named: &BTreeSet<AllocationId>) -> BTreeMap<AllocationId, GuestNetworkError> {
        let mut condemned = self.condemned.lock();
        named
            .iter()
            .filter(|alloc| condemned.insert((*alloc).clone()))
            .map(|alloc| (alloc.clone(), Self::refusal(GuestNetworkOperation::SharedAudit)))
            .collect()
    }

    /// The pinned component set (§ *Typed error taxonomy*), in canonical
    /// audit order.
    const COMPONENTS: [SharedGuestNetworkComponent; 9] = [
        SharedGuestNetworkComponent::Forwarder,
        SharedGuestNetworkComponent::FlowListeners,
        SharedGuestNetworkComponent::ControlListener,
        SharedGuestNetworkComponent::LegF,
        SharedGuestNetworkComponent::LegC,
        SharedGuestNetworkComponent::IpRules,
        SharedGuestNetworkComponent::IpSets,
        SharedGuestNetworkComponent::Dns,
        SharedGuestNetworkComponent::Supervisor,
    ];

    fn component_index(component: SharedGuestNetworkComponent) -> usize {
        Self::COMPONENTS
            .iter()
            .position(|candidate| *candidate == component)
            .unwrap_or_else(|| unreachable!("{component:?} is in the pinned component set"))
    }

    fn standing_audit_error(&self) -> Option<SharedGuestNetworkAuditError> {
        Self::COMPONENTS
            .into_iter()
            .find(|component| {
                self.audit_components[Self::component_index(*component)].load(Ordering::SeqCst)
            })
            .map(|component| SharedGuestNetworkAuditError {
                component,
                source: GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::SharedAudit,
                    expected: GuestNetworkFact::SharedComponent { component, healthy: true },
                    observed: Some(GuestNetworkFact::SharedComponent { component, healthy: false }),
                },
            })
    }
}

#[async_trait::async_trait]
impl GuestNetworkProvisioner for SimSharedGuestNetworkOwner {
    async fn provision(&self, _plan: &GuestNetworkPlan) -> Result<()> {
        self.record(GuestNetworkOperation::PairAttach);
        Self::result(&self.provision, GuestNetworkOperation::PairAttach)
    }

    /// Records `ForwardingActivate`; returns the standing outcome if one is
    /// scripted, else the next scripted outcome, else `Activated`.
    async fn activate(&self, _plan: &GuestNetworkPlan) -> Result<ForwardingActivation> {
        let standing = *self.standing_activation.lock();
        let outcome = if self.quiescence_holds() > 0 {
            ForwardingActivation::QuiescenceLatched
        } else {
            standing.unwrap_or_else(|| {
                self.activations.lock().pop_front().unwrap_or(ForwardingActivation::Activated)
            })
        };
        {
            let mut trace = self.trace.lock();
            trace.calls.push(GuestNetworkOperation::ForwardingActivate);
            trace.activations.push(outcome);
        }
        Ok(outcome)
    }

    async fn teardown(&self, _plan: &GuestNetworkPlan) -> Result<()> {
        self.record(GuestNetworkOperation::PairAbort);
        Self::result(&self.teardown, GuestNetworkOperation::PairAbort)
    }
}

#[async_trait::async_trait]
impl GuestStreamDial for SimSharedGuestNetworkOwner {
    /// Serves no allocation: every dial is `NotServing` with no state change.
    /// Consumers that need scripted streams use `SimGuestStreamDial`.
    async fn dial(
        &self,
        target: SocketAddrV4,
        purpose: DialPurpose,
    ) -> std::result::Result<GuestStream, GuestDialError> {
        self.trace.lock().dials.push(SimSharedGuestNetworkDial { target, purpose });
        Err(GuestDialError::NotServing)
    }
}

#[async_trait::async_trait]
impl SharedGuestNetworkOwner for SimSharedGuestNetworkOwner {
    async fn probe_startup(&self) -> Result<()> {
        self.record(GuestNetworkOperation::StartupProbe);
        let scripted_error = self.probe_error.lock().take();
        if let Some(error) = scripted_error {
            return Err(error);
        }
        Self::result(&self.probe, GuestNetworkOperation::StartupProbe)
    }

    async fn sweep_stale(&self) -> Result<()> {
        if let Some(host) = &self.sweep_host_state {
            let snapshot = host.observe().await.map_err(|source| GuestNetworkError::Io {
                operation: GuestNetworkOperation::CleanupComplement,
                source,
            })?;
            let mut trace = self.trace.lock();
            let call_index = trace.calls.len();
            trace.calls.push(GuestNetworkOperation::CleanupComplement);
            trace.sweep_calls.push(SimSharedGuestNetworkSweepCall { call_index, host: snapshot });
        } else {
            self.record(GuestNetworkOperation::CleanupComplement);
        }
        Self::result(&self.sweep, GuestNetworkOperation::CleanupComplement)
    }

    async fn converge_shared(&self) -> Result<()> {
        self.record(GuestNetworkOperation::ForwarderOpen);
        let result = Self::result(&self.converge, GuestNetworkOperation::ForwarderOpen);
        if result.is_ok() {
            self.converged.store(true, Ordering::SeqCst);
        }
        result
    }

    /// The double binds nothing: from its first `Ok` `converge_shared` it
    /// reports the production default ports (1240 / 1243), constant after.
    fn flow_ports(&self) -> Option<GuestFlowPorts> {
        if !self.converged.load(Ordering::SeqCst) {
            return None;
        }
        GuestFlowPorts::new(GUEST_FLOW_STREAM_PORT, GUEST_FLOW_CONTROL_PORT).ok()
    }

    /// One-shot error first, then the first armed standing component in
    /// canonical order; otherwise `Ok` naming each scripted damaged
    /// allocation not yet condemned.
    async fn audit_shared(
        &self,
    ) -> std::result::Result<SharedGuestNetworkAudit, SharedGuestNetworkAuditError> {
        self.record(GuestNetworkOperation::SharedAudit);
        let taken = self.audit_error.lock().take();
        if let Some((component, source)) = taken {
            return Err(SharedGuestNetworkAuditError { component, source });
        }
        if let Some(error) = self.standing_audit_error() {
            return Err(error);
        }
        let damage = self.audit_damage.lock().clone();
        Ok(SharedGuestNetworkAudit { damaged: self.condemn(&damage) })
    }

    /// QHOLD: a hold of this adapter's ledger; the count rises.
    async fn quiesce(&self) -> (QuiescenceHold, ForwardingQuiescence) {
        self.record(GuestNetworkOperation::ForwardingQuiesce);
        let hold = self.quiescence.ledger.issue();
        self.quiescence.holds.fetch_add(1, Ordering::SeqCst);
        (hold, ForwardingQuiescence::default())
    }

    /// QHOLD: redeems `hold` at this adapter's ledger; a foreign hold is
    /// refused and changes nothing (D-UP-4, PD-1).
    async fn restore(
        &self,
        hold: QuiescenceHold,
    ) -> std::result::Result<ForwardingRestore, QuiescenceRestoreError> {
        self.record(GuestNetworkOperation::ForwardingRestore);
        match self.quiescence.ledger.redeem(hold) {
            Ok(()) => {
                let left = self.quiescence.holds.fetch_sub(1, Ordering::SeqCst) - 1;
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
    }
}

/// Construct the accepted shared-owner + paired-EXEC inputs for injected
/// production-composition tests.
pub fn test_wiring(
    clock: Arc<dyn Clock>,
) -> (Arc<dyn SharedGuestNetworkOwner>, GuestNetworkExecWiring) {
    (Arc::new(SimSharedGuestNetworkOwner::default()), GuestNetworkExecWiring::new(clock))
}

#[cfg(test)]
#[allow(
    clippy::doc_markdown,
    clippy::expect_used,
    reason = "test contracts require exact CONTRACT_SHAPE markers and diagnostic assertions"
)]
mod tests {
    use super::*;
    use crate::adapters::clock::SimClock;
    use std::path::PathBuf;

    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// The scripted `OwnerCgroupPlacement` startup refusal (D24) is one-shot
    /// and only port calls are recorded.
    #[tokio::test]
    async fn scripted_probe_error_is_one_shot_and_records_only_port_calls() {
        let owner = SimSharedGuestNetworkOwner::default();
        owner.script_next_probe_error(GuestNetworkError::OwnerCgroupPlacement {
            owner: PathBuf::from("/sys/fs/cgroup/user.slice"),
            attach_point: PathBuf::from("/sys/fs/cgroup/overdrive.slice"),
        });

        assert!(matches!(
            owner.probe_startup().await,
            Err(GuestNetworkError::OwnerCgroupPlacement { .. })
        ));
        owner.probe_startup().await.expect("one-shot script is consumed");
        assert_eq!(
            owner.calls(),
            [GuestNetworkOperation::StartupProbe, GuestNetworkOperation::StartupProbe]
        );
    }

    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// The standing non-audit slots (sweep, converge) and the Forwarder audit
    /// slot refuse every call while armed with the mapped operation and a
    /// fresh `"scripted sim owner refusal"` source; each disarms
    /// independently, and every call is recorded once in call order.
    #[tokio::test]
    async fn standing_owner_controls_preserve_exact_operation_semantics() {
        use GuestNetworkOperation::{CleanupComplement, ForwarderOpen, SharedAudit};
        let owner = SimSharedGuestNetworkOwner::default();
        owner.script_sweep_failure(true);
        owner.script_converge_failure(true);
        owner.script_audit_failure(true);

        for call in 0..2 {
            for (result, operation) in [
                (owner.sweep_stale().await, CleanupComplement),
                (owner.converge_shared().await, ForwarderOpen),
            ] {
                assert!(
                    matches!(
                        &result,
                        Err(GuestNetworkError::Io { operation: actual, source })
                            if *actual == operation
                                && source.to_string() == "scripted sim owner refusal"
                    ),
                    "standing {operation:?} refusal on call {call}, got {result:?}",
                );
            }
        }
        let audit = owner.audit_shared().await.expect_err("Forwarder audit slot refuses");
        assert_eq!(audit.component, SharedGuestNetworkComponent::Forwarder);

        owner.script_sweep_failure(false);
        owner.script_converge_failure(false);
        owner.script_audit_failure(false);
        owner.sweep_stale().await.expect("sweep slot disarms independently");
        owner.converge_shared().await.expect("converge slot disarms independently");
        let healthy = owner.audit_shared().await.expect("audit slot disarms independently");
        assert!(healthy.damaged.is_empty(), "no damage is scripted");
        let armed = [CleanupComplement, ForwarderOpen];
        assert_eq!(
            owner.calls(),
            [
                &armed[..],
                &armed[..],
                &[SharedAudit, CleanupComplement, ForwarderOpen, SharedAudit][..],
            ]
            .concat(),
        );
    }

    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Scripted damage condemns each named allocation once: a repeated audit
    /// never names an allocation again.
    #[tokio::test]
    async fn scripted_audit_damage_condemns_each_named_allocation_once() {
        let owner = SimSharedGuestNetworkOwner::default();
        let allocs: BTreeSet<AllocationId> = ["a", "b"]
            .iter()
            .map(|name| AllocationId::new(name).expect("valid allocation id"))
            .collect();
        owner.script_audit_damage(allocs.clone());
        let first = owner.audit_shared().await.expect("no node-level audit slot is armed");
        assert_eq!(first.damaged.keys().cloned().collect::<BTreeSet<_>>(), allocs);
        let again = owner.audit_shared().await.expect("the damage set is standing");
        assert!(again.damaged.is_empty(), "a condemned allocation is never named again");
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn every_component_slot_and_exact_one_shot_obey_canonical_audit_order() {
        for component in SimSharedGuestNetworkOwner::COMPONENTS {
            let owner = SimSharedGuestNetworkOwner::default();
            owner.script_component_audit_failure(component, true);
            let error = owner.audit_shared().await.expect_err("armed component refuses audit");
            assert_eq!(error.component, component);
            owner.script_component_audit_failure(component, false);
            owner.audit_shared().await.expect("component disarms independently");
        }

        let owner = SimSharedGuestNetworkOwner::default();
        owner.script_component_audit_failure(SharedGuestNetworkComponent::Supervisor, true);
        owner.script_component_audit_failure(SharedGuestNetworkComponent::LegC, true);
        owner.script_next_audit_error(
            SharedGuestNetworkComponent::Dns,
            GuestNetworkError::Io {
                operation: GuestNetworkOperation::SharedAudit,
                source: std::io::Error::other("final one-shot"),
            },
        );
        let one_shot = owner.audit_shared().await.expect_err("one-shot has precedence");
        assert_eq!(one_shot.component, SharedGuestNetworkComponent::Dns);
        let standing = owner.audit_shared().await.expect_err("one-shot is consumed");
        assert_eq!(standing.component, SharedGuestNetworkComponent::LegC);
        owner.script_component_audit_failure(SharedGuestNetworkComponent::LegC, false);
        let next = owner.audit_shared().await.expect_err("next canonical standing component");
        assert_eq!(next.component, SharedGuestNetworkComponent::Supervisor);
    }

    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A dial is `NotServing` and is recorded with its target and purpose.
    #[tokio::test]
    async fn every_dial_is_not_serving_and_recorded() {
        let owner = SimSharedGuestNetworkOwner::default();
        let target = SocketAddrV4::new(std::net::Ipv4Addr::new(100, 95, 0, 7), 8080);
        let outcome = owner.dial(target, DialPurpose::Probe).await;
        assert!(matches!(outcome, Err(GuestDialError::NotServing)));
        assert_eq!(
            owner.dials(),
            [SimSharedGuestNetworkDial { target, purpose: DialPurpose::Probe }]
        );
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn test_wiring_returns_one_owner_port_and_one_boot_closed_exec_pair() {
        let (owner, wiring) = test_wiring(Arc::new(SimClock::new()));
        assert!(wiring.supervisor().is_boot_closed());
        assert_eq!(Arc::strong_count(&owner), 1);
    }

    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// QHOLD over the adapter's own ledger: two holds count two; the first
    /// restore is `StillQuiesced`, the second `Reopened`; a second ledger's hold is
    /// refused `ForeignHold`, changes no count and leaks at its issuing
    /// ledger; a dropped own hold keeps the count and is counted leaked.
    #[tokio::test]
    #[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
    async fn quiescence_counts_holds_of_its_own_ledger_and_refuses_a_foreign_one() {
        let owner = SimSharedGuestNetworkOwner::default();
        let (first, _) = owner.quiesce().await;
        let (second, _) = owner.quiesce().await;
        assert_eq!(owner.quiescence_holds(), 2);

        let foreign_ledger = QuiescenceLedger::new();
        let refused = owner.restore(foreign_ledger.issue()).await;
        assert_eq!(refused, Err(QuiescenceRestoreError::ForeignHold));
        assert_eq!(owner.quiescence_holds(), 2, "a foreign hold changes no count");
        assert_eq!(foreign_ledger.leaked(), 1, "it leaks at its issuing ledger");
        assert_eq!(owner.leaked_quiescence_holds(), 0, "and not at this one");

        assert_eq!(owner.restore(first).await, Ok(ForwardingRestore::StillQuiesced));
        assert_eq!(owner.restore(second).await, Ok(ForwardingRestore::Reopened));
        assert_eq!(owner.quiescence_holds(), 0);

        let (dropped, _) = owner.quiesce().await;
        drop(dropped);
        assert_eq!(owner.quiescence_holds(), 1, "a dropped hold keeps its count (fail closed)");
        assert_eq!(owner.leaked_quiescence_holds(), 1, "and is counted leaked");
    }
}
