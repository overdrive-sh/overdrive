//! Socket-free simulation adapter for the GH #295 shared guest-network owner.

#![allow(
    clippy::result_large_err,
    reason = "the sim implements the exact accepted control-plane GuestNetworkError contract"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use overdrive_control_plane::guest_network::{
    GuestNetworkError, GuestNetworkOperation, GuestNetworkPlan, GuestNetworkProvisioner, Result,
    SharedGuestNetworkAuditError, SharedGuestNetworkOwner,
};
use overdrive_core::guest_network::{GuestNetworkExecWiring, SharedGuestNetworkComponent};
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

/// Sim owner with one standing failure slot per accepted owner operation.
#[derive(Debug, Default)]
pub struct SimSharedGuestNetworkOwner {
    provision: AtomicBool,
    teardown: AtomicBool,
    probe: AtomicBool,
    sweep: AtomicBool,
    converge: AtomicBool,
    audit_components: [AtomicBool; 12],
    quiesce: AtomicBool,
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
    pub fn script_quiesce_failure(&self, armed: bool) {
        self.quiesce.store(armed, Ordering::SeqCst);
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
        if armed.load(Ordering::SeqCst) {
            Err(GuestNetworkError::Io {
                operation,
                source: std::io::Error::other("scripted sim owner refusal"),
            })
        } else {
            Ok(())
        }
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

    async fn activate(&self, _plan: &GuestNetworkPlan) -> Result<()> {
        self.record(GuestNetworkOperation::TapSetUp);
        Self::result(&self.provision, GuestNetworkOperation::TapSetUp)
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

    async fn audit_shared(&self) -> std::result::Result<(), SharedGuestNetworkAuditError> {
        self.record(GuestNetworkOperation::BridgeObserve);
        let taken = self.audit_error.lock().take();
        if let Some((component, source)) = taken {
            return Err(SharedGuestNetworkAuditError { component, source });
        }
        self.standing_audit_error().map_or(Ok(()), Err)
    }

    async fn quiesce_managed_taps(&self) -> Result<()> {
        self.record(GuestNetworkOperation::TapSetDown);
        Self::result(&self.quiesce, GuestNetworkOperation::TapSetDown)
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

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn standing_owner_controls_preserve_exact_operation_semantics() {
        let owner = SimSharedGuestNetworkOwner::default();
        owner.script_sweep_failure(true);
        owner.script_converge_failure(true);
        owner.script_audit_failure(true);
        owner.script_quiesce_failure(true);

        for (result, operation) in [
            (owner.sweep_stale().await, GuestNetworkOperation::CleanupComplement),
            (owner.converge_shared().await, GuestNetworkOperation::BridgeConverge),
            (owner.quiesce_managed_taps().await, GuestNetworkOperation::TapSetDown),
        ] {
            assert!(matches!(
                result,
                Err(GuestNetworkError::Io { operation: actual, .. }) if actual == operation
            ));
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
        assert_eq!(
            owner.calls(),
            [
                GuestNetworkOperation::CleanupComplement,
                GuestNetworkOperation::BridgeConverge,
                GuestNetworkOperation::TapSetDown,
                GuestNetworkOperation::BridgeObserve,
            ]
        );

        owner.script_sweep_failure(false);
        owner.script_converge_failure(false);
        owner.script_audit_failure(false);
        owner.script_quiesce_failure(false);
        owner.sweep_stale().await.expect("sweep slot disarms independently");
        owner.converge_shared().await.expect("converge slot disarms independently");
        owner.audit_shared().await.expect("audit slot disarms independently");
        owner.quiesce_managed_taps().await.expect("quiesce slot disarms independently");
        assert_eq!(owner.calls().len(), 8, "successful calls append after failed calls");
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
    #[ignore = "pending DELIVER step for paired BootClosed EXEC state"]
    fn test_wiring_returns_one_owner_port_and_one_boot_closed_exec_pair() {
        let (owner, wiring) = test_wiring(Arc::new(SimClock::new()));
        assert!(wiring.supervisor().is_boot_closed());
        assert_eq!(Arc::strong_count(&owner), 1);
    }
}
