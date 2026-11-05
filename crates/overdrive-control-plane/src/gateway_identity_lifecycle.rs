//! Canonical gateway-SVID reconciler registration and the lifecycle-control
//! adapter (ADR-0225).
//!
//! `gateway_svid_lifecycle_registration` is the one factory of the reconciler's
//! name and target; `boot_server` (step G0) registers it and the lifecycle
//! control reuses the same evaluation for ensure, reissue and disable wakes.
//!
#![allow(
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::redundant_pub_crate,
    dead_code,
    reason = "the control has no production caller until the `boot_server` G0 enabled boot composition lands (feature-delta.md:2970)"
)]

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

#[cfg(all(test, feature = "integration-tests"))]
mod quint_gateway_identity;

/// The canonical registration: the reconciler and the one `node/<node-id>`
/// evaluation every wake of it submits.
#[must_use]
pub(crate) fn gateway_svid_lifecycle_registration(node_id: &NodeId) -> (AnyReconciler, Evaluation) {
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
pub(crate) struct ControlPlaneGatewayIdentityLifecycle {
    target: Arc<dyn GatewayIdentityLifecycleTarget>,
    broker: Arc<Mutex<EvaluationBroker>>,
    evaluation: Evaluation,
    clock: Arc<dyn Clock>,
}

impl ControlPlaneGatewayIdentityLifecycle {
    pub(crate) fn new(
        target: Arc<dyn GatewayIdentityLifecycleTarget>,
        broker: Arc<Mutex<EvaluationBroker>>,
        evaluation: Evaluation,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self { target, broker, evaluation, clock }
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
        self.broker.lock().submit(
            self.evaluation.clone(),
            self.clock.now(),
            EvaluationEligibility::Immediate,
        );
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
