//! Canonical gateway-SVID reconciler registration and the lifecycle-control
//! adapter (ADR-0225).
//!
//! `gateway_svid_lifecycle_registration` is the one factory of the reconciler's
//! name and target; `boot_server` (step G0) registers it and the lifecycle
//! control reuses the same evaluation for ensure, reissue and disable wakes.

use std::sync::Arc;
use std::time::Instant;

use overdrive_core::eval_broker::{Evaluation, EvaluationBroker, EvaluationEligibility};
use overdrive_core::gateway_identity::{GatewayIdentityFacts, GatewayIdentityIssueRecord};
use overdrive_core::id::NodeId;
use overdrive_core::reconcilers::{Reconciler, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::wall_clock::UnixInstant;
use overdrive_gateway::ports::{
    GatewayIdentityLifecycleControl, GatewayIdentityLifecycleError, GatewayIdentityLifecycleTarget,
};
use overdrive_reconcilers::{AnyReconciler, GatewaySvidLifecycle};

use parking_lot::Mutex;

use crate::reconciler_runtime::ReconcilerRuntime;

#[cfg(all(test, feature = "integration-tests"))]
mod quint_gateway_identity;

/// The canonical registration: the reconciler and the one `node/<node-id>`
/// evaluation every wake of it submits.
#[must_use]
pub fn gateway_svid_lifecycle_registration(node_id: &NodeId) -> (AnyReconciler, Evaluation) {
    let reconciler = GatewaySvidLifecycle::canonical();
    let target = TargetResource::new(&format!("node/{node_id}")).unwrap_or_else(|_| {
        unreachable!("NodeId is non-empty and node/ is a canonical target prefix")
    });
    let evaluation = Evaluation { reconciler: reconciler.name().clone(), target };
    (AnyReconciler::GatewaySvidLifecycle(reconciler), evaluation)
}

/// The production `GatewayIdentityLifecycleControl`: submits the retained
/// evaluation through the real broker and races the slot's watch against the
/// injected clock.
pub struct ControlPlaneGatewayIdentityLifecycle {
    target: Arc<dyn GatewayIdentityLifecycleTarget>,
    broker: Arc<dyn WakeSubmit>,
    evaluation: Evaluation,
    clock: Arc<dyn Clock>,
}

/// Where the retained evaluation is submitted: the reconciler runtime's own
/// broker (the one the convergence loop drains) in production, a standalone
/// broker where a harness drains it itself.
trait WakeSubmit: Send + Sync {
    fn submit(&self, evaluation: Evaluation, now: Instant);
}

impl WakeSubmit for ReconcilerRuntime {
    fn submit(&self, evaluation: Evaluation, now: Instant) {
        self.submit_evaluation(evaluation, now, EvaluationEligibility::Immediate);
    }
}

impl WakeSubmit for Mutex<EvaluationBroker> {
    fn submit(&self, evaluation: Evaluation, now: Instant) {
        self.lock().submit(evaluation, now, EvaluationEligibility::Immediate);
    }
}

impl ControlPlaneGatewayIdentityLifecycle {
    #[cfg(all(test, feature = "integration-tests"))]
    pub(crate) fn new(
        target: Arc<dyn GatewayIdentityLifecycleTarget>,
        broker: Arc<Mutex<EvaluationBroker>>,
        evaluation: Evaluation,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self { target, broker, evaluation, clock }
    }

    /// The production control: wakes submit into the runtime's own broker.
    pub(crate) fn over_runtime(
        target: Arc<dyn GatewayIdentityLifecycleTarget>,
        runtime: Arc<ReconcilerRuntime>,
        evaluation: Evaluation,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self { target, broker: runtime, evaluation, clock }
    }
}

impl ControlPlaneGatewayIdentityLifecycle {
    /// The held facts when they are still time-usable at the injected clock.
    fn usable_current(&self) -> Option<GatewayIdentityFacts> {
        let now = UnixInstant::from_clock(self.clock.as_ref());
        self.target.current().filter(|facts| facts.not_after > now)
    }
}

#[async_trait::async_trait]
impl GatewayIdentityLifecycleControl for ControlPlaneGatewayIdentityLifecycle {
    async fn ensure_current(
        &self,
        deadline: Instant,
    ) -> Result<GatewayIdentityFacts, GatewayIdentityLifecycleError> {
        let mut receiver = self.target.subscribe();
        // The retained evaluation is submitted on the first poll even when a
        // time-usable identity is already held: the broker coalesces the wake
        // and the reconciler owns whether a near-expiry reissue is due.
        self.broker.submit(self.evaluation.clone(), self.clock.now());
        if let Some(facts) = self.usable_current() {
            return Ok(facts);
        }
        let remaining = deadline.saturating_duration_since(self.clock.now());
        let timeout = self.clock.sleep(remaining);
        tokio::pin!(timeout);
        loop {
            tokio::select! {
                changed = receiver.changed() => {
                    changed.map_err(|_| GatewayIdentityLifecycleError::Closed)?;
                    if let Some(facts) = self.usable_current() {
                        return Ok(facts);
                    }
                }
                () = &mut timeout => return Err(GatewayIdentityLifecycleError::Timeout),
            }
        }
    }

    fn current(&self) -> Option<GatewayIdentityFacts> {
        self.target.current()
    }

    fn issue_record(&self) -> Option<GatewayIdentityIssueRecord> {
        self.target.issue_record()
    }

    fn subscribe(&self) -> tokio::sync::watch::Receiver<Option<GatewayIdentityFacts>> {
        self.target.subscribe()
    }

    fn disable_after_drain(&self) {
        let _disabled_epoch = self.target.begin_disable();
    }
}
