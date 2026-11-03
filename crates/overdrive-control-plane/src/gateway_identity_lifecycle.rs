//! Canonical gateway-SVID reconciler registration and the lifecycle-control
//! adapter (ADR-0225).
//!
//! `gateway_svid_lifecycle_registration` is the one factory of the reconciler's
//! name and target; `boot_server` (step G0) registers it and the lifecycle
//! control reuses the same evaluation for ensure, reissue and disable wakes.
//!
//! SCAFFOLD: true. The registration factory is the DESIGN-pinned body; the
//! lifecycle-control behaviour is DELIVER's and fails with SCAFFOLD text.

#![allow(
    clippy::todo,
    clippy::unused_async,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::redundant_pub_crate,
    dead_code,
    reason = "public-ingress DISTILL scaffold: the control behaviour is DELIVER's and has no production caller until the enabled boot composition lands"
)]

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

#[async_trait::async_trait]
impl GatewayIdentityLifecycleControl for ControlPlaneGatewayIdentityLifecycle {
    async fn ensure_current(
        &self,
        _deadline: Instant,
    ) -> Result<GatewayIdentityFacts, GatewayIdentityLifecycleError> {
        todo!("SCAFFOLD: ControlPlaneGatewayIdentityLifecycle::ensure_current")
    }

    fn current(&self) -> Option<GatewayIdentityFacts> {
        todo!("SCAFFOLD: ControlPlaneGatewayIdentityLifecycle::current")
    }

    fn subscribe(&self) -> tokio::sync::watch::Receiver<Option<GatewayIdentityFacts>> {
        todo!("SCAFFOLD: ControlPlaneGatewayIdentityLifecycle::subscribe")
    }

    async fn disable_after_drain(
        &self,
        _deadline: Instant,
    ) -> Result<(), GatewayIdentityLifecycleError> {
        todo!("SCAFFOLD: ControlPlaneGatewayIdentityLifecycle::disable_after_drain")
    }
}
