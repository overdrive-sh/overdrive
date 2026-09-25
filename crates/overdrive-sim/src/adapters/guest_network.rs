//! Socket-free simulation adapter for the GH #295 shared guest-network owner.

#![allow(
    clippy::result_large_err,
    reason = "the sim implements the exact accepted control-plane GuestNetworkError contract"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use overdrive_control_plane::guest_network::{
    GuestLinkKind, GuestNetworkError, GuestNetworkFact, GuestNetworkOperation, GuestNetworkPlan,
    GuestNetworkProvisioner, Result, SharedGuestNetworkAudit, SharedGuestNetworkAuditError,
    SharedGuestNetworkOwner, TapActivation, TapQuiescence,
};
use overdrive_core::guest_network::{GuestNetworkExecWiring, SharedGuestNetworkComponent};
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

#[derive(Debug, Default)]
struct Trace {
    calls: Vec<GuestNetworkOperation>,
    sweep_calls: Vec<SimSharedGuestNetworkSweepCall>,
}

/// Standing outcome of every subsequent `quiesce_managed_taps` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimQuiesceOutcome {
    /// `Ok(TapQuiescence)` naming these allocations unconfirmed, each with a
    /// fresh `GuestNetworkError::Io { operation: TapSetDown, .. }`; empty is
    /// full quiescence. The `Default`.
    Unconfirmed(BTreeSet<AllocationId>),
    /// `Err(GuestNetworkError::Io { operation: TapSetDown, .. })`.
    Fail,
    /// A future that never resolves.
    Hang,
}

impl Default for SimQuiesceOutcome {
    fn default() -> Self {
        Self::Unconfirmed(BTreeSet::new())
    }
}

/// The owner's private runtime-quiescence model: the latch and the
/// `Condemned` exclusion set (D-295-R5, R14).
#[derive(Debug, Default)]
struct Quiescence {
    /// Set by every `quiesce_managed_taps`; cleared only by a successful
    /// `restore_quiesced_taps`.
    latched: bool,
    /// Every allocation a returned quiescence or audit result has named.
    condemned: BTreeSet<AllocationId>,
}

/// Sim owner with one standing failure slot per accepted owner operation.
#[derive(Debug, Default)]
pub struct SimSharedGuestNetworkOwner {
    provision: AtomicBool,
    teardown: AtomicBool,
    probe: AtomicBool,
    sweep: AtomicBool,
    converge: AtomicBool,
    restore: AtomicBool,
    audit_components: [AtomicBool; 12],
    quiesce_outcome: Mutex<SimQuiesceOutcome>,
    audit_damage: Mutex<BTreeSet<AllocationId>>,
    quiescence: Mutex<Quiescence>,
    probe_error: Mutex<Option<GuestNetworkError>>,
    audit_error: Mutex<Option<(SharedGuestNetworkComponent, GuestNetworkError)>>,
    sweep_host_state: Option<SimVmHostState>,
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
    pub fn script_teardown_failure(&self, armed: bool) {
        self.teardown.store(armed, Ordering::SeqCst);
    }
    pub fn script_probe_failure(&self, armed: bool) {
        self.probe.store(armed, Ordering::SeqCst);
    }
    /// Script the next startup-probe result with the exact accepted typed
    /// error, including a primary+cleanup aggregate when required.
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
        self.script_component_audit_failure(SharedGuestNetworkComponent::Bridge, armed);
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
    /// Set the standing outcome of every subsequent `quiesce_managed_taps`.
    pub fn script_quiesce_outcome(&self, outcome: SimQuiesceOutcome) {
        *self.quiesce_outcome.lock() = outcome;
    }
    /// Arm (`true`) or disarm (`false`) the standing `restore_quiesced_taps`
    /// refusal.
    pub fn script_restore_failure(&self, armed: bool) {
        self.restore.store(armed, Ordering::SeqCst);
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

    /// The host's source-less refusal for an allocation with no activatable
    /// record — here, a condemned one.
    fn condemned_refusal(plan: &GuestNetworkPlan) -> GuestNetworkError {
        GuestNetworkError::PostconditionMismatch {
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
        }
    }

    /// Condemn every not-yet-condemned allocation of `named`, mapping each to
    /// a fresh typed error for `operation`. Already-condemned allocations are
    /// never named again.
    fn condemn(
        &self,
        named: &BTreeSet<AllocationId>,
        operation: GuestNetworkOperation,
    ) -> BTreeMap<AllocationId, GuestNetworkError> {
        let mut quiescence = self.quiescence.lock();
        named
            .iter()
            .filter(|alloc| quiescence.condemned.insert((*alloc).clone()))
            .map(|alloc| (alloc.clone(), Self::refusal(operation)))
            .collect()
    }

    const COMPONENTS: [SharedGuestNetworkComponent; 12] = [
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

    const fn component_index(component: SharedGuestNetworkComponent) -> usize {
        match component {
            SharedGuestNetworkComponent::Bridge => 0,
            SharedGuestNetworkComponent::LegF => 1,
            SharedGuestNetworkComponent::LegC => 2,
            SharedGuestNetworkComponent::Dns => 3,
            SharedGuestNetworkComponent::TcxLink => 4,
            SharedGuestNetworkComponent::EndpointMap => 5,
            SharedGuestNetworkComponent::CounterMap => 6,
            SharedGuestNetworkComponent::BpffsPin => 7,
            SharedGuestNetworkComponent::BridgeGuard => 8,
            SharedGuestNetworkComponent::IpRules => 9,
            SharedGuestNetworkComponent::IpSets => 10,
            SharedGuestNetworkComponent::Supervisor => 11,
        }
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
                    expected:
                        overdrive_control_plane::guest_network::GuestNetworkFact::SharedComponent {
                            component,
                            healthy: true,
                        },
                    observed: Some(
                        overdrive_control_plane::guest_network::GuestNetworkFact::SharedComponent {
                            component,
                            healthy: false,
                        },
                    ),
                },
            })
    }
}

#[async_trait::async_trait]
impl GuestNetworkProvisioner for SimSharedGuestNetworkOwner {
    async fn provision(&self, _plan: &GuestNetworkPlan) -> Result<()> {
        self.record(GuestNetworkOperation::TapCreate);
        Self::result(&self.provision, GuestNetworkOperation::TapCreate)
    }

    /// `Ok(QuiescenceLatched)` with nothing recorded while the latch is set;
    /// otherwise records `TapSetUp` and returns the host's source-less
    /// `PostconditionMismatch` for a condemned allocation, `Ok(Raised)` else.
    async fn activate(&self, plan: &GuestNetworkPlan) -> Result<TapActivation> {
        let condemned = {
            let quiescence = self.quiescence.lock();
            if quiescence.latched {
                return Ok(TapActivation::QuiescenceLatched);
            }
            quiescence.condemned.contains(plan.alloc())
        };
        self.record(GuestNetworkOperation::TapSetUp);
        if condemned {
            return Err(Self::condemned_refusal(plan));
        }
        Ok(TapActivation::Raised)
    }

    async fn teardown(&self, _plan: &GuestNetworkPlan) -> Result<()> {
        self.record(GuestNetworkOperation::TapDelete);
        Self::result(&self.teardown, GuestNetworkOperation::TapDelete)
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
        self.record(GuestNetworkOperation::BridgeConverge);
        Self::result(&self.converge, GuestNetworkOperation::BridgeConverge)
    }

    /// Node-level result as D11 pins it; when no node-level slot fires,
    /// `Ok` naming each scripted damaged allocation not yet condemned (each
    /// joins the condemned set).
    async fn audit_shared(
        &self,
    ) -> std::result::Result<SharedGuestNetworkAudit, SharedGuestNetworkAuditError> {
        self.record(GuestNetworkOperation::BridgeObserve);
        let taken = self.audit_error.lock().take();
        if let Some((component, source)) = taken {
            return Err(SharedGuestNetworkAuditError { component, source });
        }
        if let Some(error) = self.standing_audit_error() {
            return Err(error);
        }
        let damage = self.audit_damage.lock().clone();
        Ok(SharedGuestNetworkAudit {
            damaged: self.condemn(&damage, GuestNetworkOperation::TapObserve),
        })
    }

    /// Sets the latch, records `TapSetDown`, and returns the standing outcome.
    async fn quiesce_managed_taps(&self) -> Result<TapQuiescence> {
        self.quiescence.lock().latched = true;
        self.record(GuestNetworkOperation::TapSetDown);
        let outcome = self.quiesce_outcome.lock().clone();
        match outcome {
            SimQuiesceOutcome::Unconfirmed(named) => Ok(TapQuiescence {
                unconfirmed: self.condemn(&named, GuestNetworkOperation::TapSetDown),
            }),
            SimQuiesceOutcome::Fail => Err(Self::refusal(GuestNetworkOperation::TapSetDown)),
            SimQuiesceOutcome::Hang => std::future::pending().await,
        }
    }

    /// Records `TapSetUp`; returns the standing restore refusal, and clears
    /// the latch only on `Ok`.
    async fn restore_quiesced_taps(&self) -> Result<()> {
        self.record(GuestNetworkOperation::TapSetUp);
        Self::result(&self.restore, GuestNetworkOperation::TapSetUp)?;
        self.quiescence.lock().latched = false;
        Ok(())
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
    use futures::FutureExt;
    use overdrive_control_plane::guest_network::{GuestNetworkFact, GuestNetworkProbeStage};

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn scripted_probe_error_is_one_shot_and_records_only_port_calls() {
        let owner = SimSharedGuestNetworkOwner::default();
        owner.script_next_probe_error(GuestNetworkError::PostconditionMismatch {
            operation: GuestNetworkOperation::StartupProbe,
            expected: GuestNetworkFact::StartupProbe {
                stage: GuestNetworkProbeStage::Classifier,
                passed: true,
            },
            observed: Some(GuestNetworkFact::StartupProbe {
                stage: GuestNetworkProbeStage::Classifier,
                passed: false,
            }),
        });

        assert!(matches!(
            owner.probe_startup().await,
            Err(GuestNetworkError::PostconditionMismatch { .. })
        ));
        owner.probe_startup().await.expect("one-shot script is consumed");
        assert_eq!(
            owner.calls(),
            [GuestNetworkOperation::StartupProbe, GuestNetworkOperation::StartupProbe]
        );
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn one_shot_probe_error_replaces_earlier_value_and_preserves_the_standing_slot() {
        let owner = SimSharedGuestNetworkOwner::default();
        owner.script_probe_failure(true);
        owner.script_next_probe_error(GuestNetworkError::PostconditionMismatch {
            operation: GuestNetworkOperation::StartupProbe,
            expected: GuestNetworkFact::StartupProbe {
                stage: GuestNetworkProbeStage::Classifier,
                passed: true,
            },
            observed: None,
        });
        owner.script_next_probe_error(GuestNetworkError::PostconditionMismatch {
            operation: GuestNetworkOperation::StartupProbe,
            expected: GuestNetworkFact::StartupProbe {
                stage: GuestNetworkProbeStage::OriginalDestination,
                passed: true,
            },
            observed: Some(GuestNetworkFact::StartupProbe {
                stage: GuestNetworkProbeStage::OriginalDestination,
                passed: false,
            }),
        });

        let first = owner.probe_startup().await.expect_err("replacement one-shot takes precedence");
        assert!(matches!(
            first,
            GuestNetworkError::PostconditionMismatch {
                expected: GuestNetworkFact::StartupProbe {
                    stage: GuestNetworkProbeStage::OriginalDestination,
                    ..
                },
                ..
            }
        ));
        assert!(matches!(
            owner.probe_startup().await,
            Err(GuestNetworkError::Io { operation: GuestNetworkOperation::StartupProbe, .. })
        ));
        owner.script_probe_failure(false);
        owner.probe_startup().await.expect("disarming the standing slot restores success");
        assert_eq!(
            owner.calls(),
            [
                GuestNetworkOperation::StartupProbe,
                GuestNetworkOperation::StartupProbe,
                GuestNetworkOperation::StartupProbe,
            ]
        );
        let first_snapshot = owner.calls();
        assert_eq!(owner.calls(), first_snapshot, "call snapshots are ordered and non-draining");
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH.
    /// S-ND295-53 — Activation waits out a recovery and never turns it into a failure.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// The standing non-audit slots (sweep, converge, restore), the standing
    /// quiescence outcome `Fail`, and the Bridge audit slot each refuse every
    /// call while armed, with the exact mapped operation and a fresh
    /// `"scripted sim owner refusal"` source; each disarms independently, the
    /// default quiescence outcome is full quiescence, and every call is
    /// recorded once in call order (FD 6705-6760).
    #[tokio::test]
    async fn standing_owner_controls_preserve_exact_operation_semantics() {
        use GuestNetworkOperation::{
            BridgeConverge, BridgeObserve, CleanupComplement, TapSetDown, TapSetUp,
        };
        let owner = SimSharedGuestNetworkOwner::default();
        owner.script_sweep_failure(true);
        owner.script_converge_failure(true);
        owner.script_audit_failure(true);
        owner.script_quiesce_outcome(SimQuiesceOutcome::Fail);
        owner.script_restore_failure(true);

        for call in 0..2 {
            for (result, operation) in [
                (owner.sweep_stale().await, CleanupComplement),
                (owner.converge_shared().await, BridgeConverge),
                (owner.quiesce_managed_taps().await.map(|_| ()), TapSetDown),
                (owner.restore_quiesced_taps().await, TapSetUp),
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
        let audit = owner.audit_shared().await.expect_err("Bridge audit slot refuses");
        assert_eq!(audit.component, SharedGuestNetworkComponent::Bridge);
        assert!(matches!(
            audit.source,
            GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::SharedAudit,
                ..
            }
        ));
        let armed = [CleanupComplement, BridgeConverge, TapSetDown, TapSetUp];
        assert_eq!(owner.calls(), [&armed[..], &armed[..], &[BridgeObserve][..]].concat());

        owner.script_sweep_failure(false);
        owner.script_converge_failure(false);
        owner.script_audit_failure(false);
        owner.script_quiesce_outcome(SimQuiesceOutcome::default());
        owner.script_restore_failure(false);
        owner.sweep_stale().await.expect("sweep slot disarms independently");
        owner.converge_shared().await.expect("converge slot disarms independently");
        let healthy = owner.audit_shared().await.expect("audit slot disarms independently");
        assert!(healthy.damaged.is_empty(), "no damage is scripted");
        let quiesced = owner.quiesce_managed_taps().await.expect("quiesce outcome resets");
        assert!(quiesced.unconfirmed.is_empty(), "the default outcome is full quiescence");
        owner.restore_quiesced_taps().await.expect("restore slot disarms independently");
        assert_eq!(
            owner.calls(),
            [
                &armed[..],
                &armed[..],
                &[
                    BridgeObserve,
                    CleanupComplement,
                    BridgeConverge,
                    BridgeObserve,
                    TapSetDown,
                    TapSetUp
                ][..],
            ]
            .concat(),
            "successful calls append after failed calls, each recorded once",
        );
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH.
    /// S-ND295-53 — Activation waits out a recovery and never turns it into a failure.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Each `SimQuiesceOutcome` gives its pinned result: `Unconfirmed(set)`
    /// names each not-yet-condemned member with a fresh `Io { TapSetDown }`
    /// (empty is full quiescence), `Fail` refuses, `Hang` never resolves.
    /// Every allocation a quiescence or audit result names is condemned and
    /// never named again by either operation (FD 2142-2165, 6705-6718).
    #[tokio::test]
    async fn scripted_quiescence_outcomes_condemn_each_named_allocation_once() {
        use GuestNetworkOperation::{BridgeObserve, TapObserve, TapSetDown};
        let owner = SimSharedGuestNetworkOwner::default();

        let full = owner.quiesce_managed_taps().await.expect("the default outcome succeeds");
        assert!(full.unconfirmed.is_empty(), "the default Unconfirmed(∅) is full quiescence");

        owner.script_quiesce_outcome(SimQuiesceOutcome::Unconfirmed(allocs(&["a", "b"])));
        let first = owner.quiesce_managed_taps().await.expect("Unconfirmed is an Ok outcome");
        assert_named(&first.unconfirmed, &allocs(&["a", "b"]), TapSetDown);
        let repeat = owner.quiesce_managed_taps().await.expect("the outcome is standing");
        assert_named(&repeat.unconfirmed, &allocs(&[]), TapSetDown);

        owner.script_quiesce_outcome(SimQuiesceOutcome::Unconfirmed(allocs(&["b", "c"])));
        let overlap = owner.quiesce_managed_taps().await.expect("Unconfirmed is an Ok outcome");
        assert_named(&overlap.unconfirmed, &allocs(&["c"]), TapSetDown);

        owner.script_audit_damage(allocs(&["a", "c", "d"]));
        let audit = owner.audit_shared().await.expect("no node-level audit slot is armed");
        assert_named(&audit.damaged, &allocs(&["d"]), TapObserve);
        let reaudit = owner.audit_shared().await.expect("the damage set is standing");
        assert_named(&reaudit.damaged, &allocs(&[]), TapObserve);
        owner.script_quiesce_outcome(SimQuiesceOutcome::Unconfirmed(allocs(&["d"])));
        let after_audit = owner.quiesce_managed_taps().await.expect("Unconfirmed is an Ok outcome");
        assert_named(&after_audit.unconfirmed, &allocs(&[]), TapSetDown);

        owner.script_quiesce_outcome(SimQuiesceOutcome::Fail);
        let failed = owner.quiesce_managed_taps().await;
        assert!(
            matches!(&failed, Err(GuestNetworkError::Io { operation: TapSetDown, .. })),
            "Fail refuses with Io {{ TapSetDown }}, got {failed:?}",
        );

        owner.script_quiesce_outcome(SimQuiesceOutcome::Hang);
        let mut hung = owner.quiesce_managed_taps();
        for poll in 0..2 {
            assert!(hung.as_mut().now_or_never().is_none(), "Hang never resolves (poll {poll})");
            tokio::task::yield_now().await;
        }
        drop(hung);

        owner.script_quiesce_outcome(SimQuiesceOutcome::Unconfirmed(allocs(&["e"])));
        let fresh = owner.quiesce_managed_taps().await.expect("Unconfirmed is an Ok outcome");
        assert_named(&fresh.unconfirmed, &allocs(&["e"]), TapSetDown);

        assert_eq!(
            owner.calls(),
            [
                TapSetDown,
                TapSetDown,
                TapSetDown,
                TapSetDown,
                BridgeObserve,
                BridgeObserve,
                TapSetDown,
                TapSetDown,
                TapSetDown,
                TapSetDown,
            ],
            "every quiescence records TapSetDown and every audit BridgeObserve, hung call included",
        );
    }

    /// The allocation ids `names`, as the set a scripted outcome takes.
    fn allocs(names: &[&str]) -> BTreeSet<AllocationId> {
        names.iter().map(|name| AllocationId::new(name).expect("valid allocation id")).collect()
    }

    /// `named` names exactly `expected`, each with a fresh `Io { operation }`.
    fn assert_named(
        named: &BTreeMap<AllocationId, GuestNetworkError>,
        expected: &BTreeSet<AllocationId>,
        operation: GuestNetworkOperation,
    ) {
        assert_eq!(named.keys().cloned().collect::<BTreeSet<_>>(), *expected);
        for (alloc, error) in named {
            assert!(
                matches!(error, GuestNetworkError::Io { operation: actual, .. } if *actual == operation),
                "{alloc} is named with Io {{ {operation:?} }}, got {error:?}",
            );
        }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn every_component_slot_and_exact_one_shot_obey_canonical_audit_order() {
        for component in SimSharedGuestNetworkOwner::COMPONENTS {
            let owner = SimSharedGuestNetworkOwner::default();
            owner.script_component_audit_failure(component, true);
            let error = owner.audit_shared().await.expect_err("armed component refuses audit");
            assert_eq!(error.component, component);
            assert!(matches!(
                error.source,
                GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::SharedAudit,
                    expected: GuestNetworkFact::SharedComponent {
                        component: expected,
                        healthy: true,
                    },
                    observed: Some(GuestNetworkFact::SharedComponent {
                        component: observed,
                        healthy: false,
                    }),
                } if expected == component && observed == component
            ));
            owner.script_component_audit_failure(component, false);
            owner.audit_shared().await.expect("component disarms independently");
            assert_eq!(
                owner.calls(),
                [GuestNetworkOperation::BridgeObserve, GuestNetworkOperation::BridgeObserve]
            );
        }

        let owner = SimSharedGuestNetworkOwner::default();
        owner.script_component_audit_failure(SharedGuestNetworkComponent::Supervisor, true);
        owner.script_component_audit_failure(SharedGuestNetworkComponent::LegC, true);
        owner.script_next_audit_error(
            SharedGuestNetworkComponent::TcxLink,
            GuestNetworkError::Io {
                operation: GuestNetworkOperation::TcxQuery,
                source: std::io::Error::other("replaced one-shot"),
            },
        );
        owner.script_next_audit_error(
            SharedGuestNetworkComponent::Dns,
            GuestNetworkError::Io {
                operation: GuestNetworkOperation::BridgeObserve,
                source: std::io::Error::other("final one-shot"),
            },
        );
        let one_shot = owner.audit_shared().await.expect_err("one-shot has precedence");
        assert_eq!(one_shot.component, SharedGuestNetworkComponent::Dns);
        assert!(matches!(
            one_shot.source,
            GuestNetworkError::Io {
                operation: GuestNetworkOperation::BridgeObserve,
                source,
            } if source.to_string() == "final one-shot"
        ));
        let standing = owner.audit_shared().await.expect_err("one-shot is consumed");
        assert_eq!(standing.component, SharedGuestNetworkComponent::LegC);
        owner.script_component_audit_failure(SharedGuestNetworkComponent::LegC, false);
        let next = owner.audit_shared().await.expect_err("next canonical standing component");
        assert_eq!(next.component, SharedGuestNetworkComponent::Supervisor);
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn test_wiring_returns_one_owner_port_and_one_boot_closed_exec_pair() {
        let (owner, wiring) = test_wiring(Arc::new(SimClock::new()));
        assert!(wiring.supervisor().is_boot_closed());
        assert_eq!(Arc::strong_count(&owner), 1);
    }
}
