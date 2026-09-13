//! Sim bindings for the dedicated gateway identity and exact-peer mTLS ports.
//!
//! SCAFFOLD: true. DISTILL pins the precise adapter surface consumed by the
//! seeded Gateway Application compositions; DELIVER replaces the RED bodies.

#![expect(clippy::todo, reason = "public-ingress Sim adapter RED scaffolds")]

use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::Instant;

use overdrive_core::eval_broker::{Evaluation, EvaluationBroker};
use overdrive_core::gateway_identity::{
    GatewayIdentityDesired, GatewayIdentityEpoch, GatewayIdentityEpochError, GatewayIdentityFacts,
};
use overdrive_core::id::SpiffeId;
use overdrive_core::traits::ca::SvidMaterial;
use overdrive_core::traits::clock::Clock;
use overdrive_gateway::ports::{
    GatewayClientMtls, GatewayClientMtlsError, GatewayIdentityActionTarget,
    GatewayIdentityLifecycleControl, GatewayIdentityLifecycleError, GatewayIdentityLifecycleTarget,
    GatewayIdentityMutationOutcome, GatewayUpstream, GatewayUpstreamSeal,
};
use parking_lot::Mutex;
use tokio::sync::watch;

/// Scripted exact-peer adapter that can construct an upstream only with the
/// unforgeable seal transferred by the gateway builder.
pub struct SimGatewayClientMtls {
    _seal: GatewayUpstreamSeal,
    _clock: Arc<dyn Clock>,
}

impl SimGatewayClientMtls {
    pub fn new(_seal: GatewayUpstreamSeal, _clock: Arc<dyn Clock>) -> Self {
        todo!("SCAFFOLD: SimGatewayClientMtls::new")
    }

    pub fn set_peer(&self, _peer: SpiffeId) {
        todo!("SCAFFOLD: SimGatewayClientMtls::set_peer")
    }
}

#[async_trait::async_trait]
impl GatewayClientMtls for SimGatewayClientMtls {
    async fn authenticate(
        &self,
        _connected: OwnedFd,
        _expected_peer: SpiffeId,
        _deadline: Instant,
    ) -> Result<GatewayUpstream, GatewayClientMtlsError> {
        todo!("SCAFFOLD: SimGatewayClientMtls::authenticate exact peer")
    }
}

/// In-memory checked-epoch target containing only desired/current gateway
/// identity facts.
pub struct SimGatewayIdentityActionTarget {
    _private: (),
}

impl SimGatewayIdentityActionTarget {
    pub fn new(_desired: GatewayIdentityDesired) -> Self {
        todo!("SCAFFOLD: SimGatewayIdentityActionTarget::new")
    }

    pub fn current(&self) -> Option<GatewayIdentityFacts> {
        todo!("SCAFFOLD: SimGatewayIdentityActionTarget::current")
    }

    pub fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>> {
        todo!("SCAFFOLD: SimGatewayIdentityActionTarget::subscribe")
    }

    pub fn begin_disable(&self) -> Result<GatewayIdentityEpoch, GatewayIdentityEpochError> {
        todo!("SCAFFOLD: SimGatewayIdentityActionTarget::begin_disable")
    }
}

impl GatewayIdentityActionTarget for SimGatewayIdentityActionTarget {
    fn hold_audited(
        &self,
        _epoch: GatewayIdentityEpoch,
        _material: SvidMaterial,
    ) -> GatewayIdentityMutationOutcome {
        todo!("SCAFFOLD: Sim gateway identity epoch-checked hold")
    }

    fn drop_after_drain(&self, _epoch: GatewayIdentityEpoch) -> GatewayIdentityMutationOutcome {
        todo!("SCAFFOLD: Sim gateway identity epoch-checked drop")
    }
}

impl GatewayIdentityLifecycleTarget for SimGatewayIdentityActionTarget {
    fn current(&self) -> Option<GatewayIdentityFacts> {
        self.current()
    }

    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>> {
        self.subscribe()
    }

    fn begin_disable(&self) -> Result<GatewayIdentityEpoch, GatewayIdentityEpochError> {
        self.begin_disable()
    }
}

/// Lifecycle-control adapter over the real evaluation broker and Sim identity
/// target.
pub struct SimGatewayIdentityLifecycleControl {
    _target: Arc<SimGatewayIdentityActionTarget>,
    _broker: Arc<Mutex<EvaluationBroker>>,
    _evaluation: Evaluation,
    _clock: Arc<dyn Clock>,
}

impl SimGatewayIdentityLifecycleControl {
    pub fn new(
        target: Arc<SimGatewayIdentityActionTarget>,
        broker: Arc<Mutex<EvaluationBroker>>,
        evaluation: Evaluation,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self { _target: target, _broker: broker, _evaluation: evaluation, _clock: clock }
    }
}

#[async_trait::async_trait]
impl GatewayIdentityLifecycleControl for SimGatewayIdentityLifecycleControl {
    async fn ensure_current(
        &self,
        _deadline: Instant,
    ) -> Result<GatewayIdentityFacts, GatewayIdentityLifecycleError> {
        todo!("SCAFFOLD: Sim gateway identity evaluation and current wait")
    }

    fn current(&self) -> Option<GatewayIdentityFacts> {
        todo!("SCAFFOLD: Sim gateway identity lifecycle current")
    }

    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>> {
        todo!("SCAFFOLD: Sim gateway identity lifecycle subscribe")
    }

    async fn disable_after_drain(
        &self,
        _deadline: Instant,
    ) -> Result<(), GatewayIdentityLifecycleError> {
        todo!("SCAFFOLD: Sim gateway identity disable after drain")
    }
}
