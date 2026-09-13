//! Canonical gateway-SVID reconciler registration and lifecycle-control adapter.
//!
//! SCAFFOLD: true. Names/targets and the exact constructor are production
//! callers now; DELIVER replaces the marked broker/watch behavior.

#![expect(clippy::todo, reason = "public-ingress gateway identity RED scaffold")]
#![allow(dead_code, reason = "activated by the DELIVER enabled serve composition")]

use std::sync::Arc;
use std::time::Instant;

use overdrive_core::eval_broker::{Evaluation, EvaluationBroker};
use overdrive_core::gateway_identity::GatewayIdentityFacts;
use overdrive_core::id::NodeId;
use overdrive_core::reconcilers::{Reconciler, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_gateway::ports::{
    GatewayIdentityLifecycleControl, GatewayIdentityLifecycleError, GatewayIdentityLifecycleTarget,
};
use overdrive_reconcilers::{AnyReconciler, GatewaySvidLifecycle};
use parking_lot::Mutex;

#[must_use]
pub(crate) fn gateway_svid_lifecycle_registration(node_id: &NodeId) -> (AnyReconciler, Evaluation) {
    let reconciler = GatewaySvidLifecycle::canonical();
    let target = TargetResource::new(&format!("node/{node_id}"))
        .unwrap_or_else(|_| unreachable!("NodeId is non-empty and node/ is canonical"));
    let evaluation = Evaluation { reconciler: reconciler.name().clone(), target };
    (AnyReconciler::GatewaySvidLifecycle(reconciler), evaluation)
}

pub(crate) struct ControlPlaneGatewayIdentityLifecycle {
    _target: Arc<dyn GatewayIdentityLifecycleTarget>,
    _broker: Arc<Mutex<EvaluationBroker>>,
    _evaluation: Evaluation,
    _clock: Arc<dyn Clock>,
}

impl ControlPlaneGatewayIdentityLifecycle {
    pub(crate) fn new(
        target: Arc<dyn GatewayIdentityLifecycleTarget>,
        broker: Arc<Mutex<EvaluationBroker>>,
        evaluation: Evaluation,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self { _target: target, _broker: broker, _evaluation: evaluation, _clock: clock }
    }
}

#[async_trait::async_trait]
impl GatewayIdentityLifecycleControl for ControlPlaneGatewayIdentityLifecycle {
    async fn ensure_current(
        &self,
        _deadline: Instant,
    ) -> Result<GatewayIdentityFacts, GatewayIdentityLifecycleError> {
        todo!("SCAFFOLD: submit canonical evaluation and await exact desired epoch")
    }

    fn current(&self) -> Option<GatewayIdentityFacts> {
        todo!("SCAFFOLD: gateway lifecycle current")
    }

    fn subscribe(&self) -> tokio::sync::watch::Receiver<Option<GatewayIdentityFacts>> {
        todo!("SCAFFOLD: gateway lifecycle subscription")
    }

    async fn disable_after_drain(
        &self,
        _deadline: Instant,
    ) -> Result<(), GatewayIdentityLifecycleError> {
        todo!("SCAFFOLD: begin checked disable and await empty Current")
    }
}
