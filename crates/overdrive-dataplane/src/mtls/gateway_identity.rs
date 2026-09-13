//! Dedicated non-allocation gateway SVID slot and exact-peer client adapter.
//!
//! SCAFFOLD: true. DISTILL pins the accepted API and the real-rustls acceptance
//! bodies; DELIVER replaces only the marked RED behavior.

#![expect(clippy::todo, reason = "public-ingress DISTILL RED scaffolds")]

use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use overdrive_core::gateway_identity::{
    GatewayIdentityCurrentRead, GatewayIdentityDesired, GatewayIdentityDesiredRead,
    GatewayIdentityEpoch, GatewayIdentityEpochError, GatewayIdentityFacts,
};
use overdrive_core::id::NodeId;
use overdrive_core::traits::ca::{SvidMaterial, TrustBundle};
use overdrive_core::traits::clock::Clock;
use overdrive_gateway::ports::{
    GatewayClientMtls, GatewayClientMtlsError, GatewayIdentityActionTarget,
    GatewayIdentityLifecycleTarget, GatewayIdentityMutationOutcome, GatewayUpstream,
    GatewayUpstreamSeal,
};
use overdrive_gateway::runtime::GatewayLimits;
use tokio::sync::watch;

/// Single checked-epoch, non-allocation gateway SVID holder.
pub struct GatewayIdentitySlot {
    _private: (),
}

impl GatewayIdentitySlot {
    pub fn new(_node_id: NodeId, _trust_bundle: TrustBundle) -> Self {
        todo!("SCAFFOLD: GatewayIdentitySlot::new")
    }
    pub fn desired(&self) -> GatewayIdentityDesired {
        todo!("SCAFFOLD: GatewayIdentitySlot::desired")
    }
    pub fn current(&self) -> Option<GatewayIdentityFacts> {
        todo!("SCAFFOLD: GatewayIdentitySlot::current")
    }
    pub fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>> {
        todo!("SCAFFOLD: GatewayIdentitySlot::subscribe")
    }
    pub fn client_access(self: &Arc<Self>, _clock: Arc<dyn Clock>) -> GatewayClientIdentityAccess {
        todo!("SCAFFOLD: GatewayIdentitySlot::client_access")
    }
    pub fn action_target(self: &Arc<Self>) -> GatewayIdentityActionHandle {
        todo!("SCAFFOLD: GatewayIdentitySlot::action_target")
    }
    pub fn lifecycle_target(self: &Arc<Self>) -> GatewayIdentityLifecycleHandle {
        todo!("SCAFFOLD: GatewayIdentitySlot::lifecycle_target")
    }
    pub(crate) fn begin_disable(&self) -> Result<GatewayIdentityEpoch, GatewayIdentityEpochError> {
        todo!("SCAFFOLD: GatewayIdentitySlot::begin_disable")
    }
    pub(crate) fn hold_audited(
        &self,
        _epoch: GatewayIdentityEpoch,
        _material: SvidMaterial,
    ) -> GatewayIdentityMutationOutcome {
        todo!("SCAFFOLD: GatewayIdentitySlot::hold_audited")
    }
    pub(crate) fn drop_after_drain(
        &self,
        _epoch: GatewayIdentityEpoch,
    ) -> GatewayIdentityMutationOutcome {
        todo!("SCAFFOLD: GatewayIdentitySlot::drop_after_drain")
    }
}

impl GatewayIdentityDesiredRead for GatewayIdentitySlot {
    fn desired(&self) -> GatewayIdentityDesired {
        self.desired()
    }
}
impl GatewayIdentityCurrentRead for GatewayIdentitySlot {
    fn current(&self) -> Option<GatewayIdentityFacts> {
        self.current()
    }
}

pub struct GatewayIdentityActionHandle {
    slot: Arc<GatewayIdentitySlot>,
}
pub struct GatewayIdentityLifecycleHandle {
    slot: Arc<GatewayIdentitySlot>,
}

impl GatewayIdentityActionTarget for GatewayIdentityActionHandle {
    fn hold_audited(
        &self,
        epoch: GatewayIdentityEpoch,
        material: SvidMaterial,
    ) -> GatewayIdentityMutationOutcome {
        self.slot.hold_audited(epoch, material)
    }
    fn drop_after_drain(&self, epoch: GatewayIdentityEpoch) -> GatewayIdentityMutationOutcome {
        self.slot.drop_after_drain(epoch)
    }
}

impl GatewayIdentityLifecycleTarget for GatewayIdentityLifecycleHandle {
    fn current(&self) -> Option<GatewayIdentityFacts> {
        self.slot.current()
    }
    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>> {
        self.slot.subscribe()
    }
    fn begin_disable(&self) -> Result<GatewayIdentityEpoch, GatewayIdentityEpochError> {
        self.slot.begin_disable()
    }
}

/// Opaque borrow-only private-material capability consumed only by host mTLS.
pub struct GatewayClientIdentityAccess {
    slot: Arc<GatewayIdentitySlot>,
    clock: Arc<dyn Clock>,
}

impl GatewayClientIdentityAccess {
    #[expect(dead_code, reason = "activated by HostGatewayClientMtls in DELIVER")]
    pub(crate) fn with_current<R>(
        &self,
        _use_material: impl FnOnce(&SvidMaterial, &TrustBundle) -> Result<R, GatewayClientMtlsError>,
    ) -> Result<R, GatewayClientMtlsError> {
        let _ = (&self.slot, &self.clock);
        todo!("SCAFFOLD: GatewayClientIdentityAccess::with_current")
    }
}

/// Real rustls adapter presenting the dedicated gateway SVID and requiring the
/// receipt-selected workload SPIFFE identity.
pub struct HostGatewayClientMtls {
    _identity: GatewayClientIdentityAccess,
    _seal: GatewayUpstreamSeal,
    _limits: GatewayLimits,
}

impl HostGatewayClientMtls {
    pub fn new(
        _identity: GatewayClientIdentityAccess,
        _seal: GatewayUpstreamSeal,
        _limits: GatewayLimits,
    ) -> Self {
        todo!("SCAFFOLD: HostGatewayClientMtls::new")
    }
}

#[async_trait]
impl GatewayClientMtls for HostGatewayClientMtls {
    async fn authenticate(
        &self,
        _connected: OwnedFd,
        _expected_peer: overdrive_core::id::SpiffeId,
        _deadline: Instant,
    ) -> Result<GatewayUpstream, GatewayClientMtlsError> {
        todo!("SCAFFOLD: HostGatewayClientMtls::authenticate")
    }
}

#[cfg(test)]
mod acceptance {
    #![allow(
        clippy::doc_markdown,
        clippy::expect_used,
        reason = "acceptance fixture preconditions and Contract Shape metadata"
    )]

    use std::time::Duration;

    use overdrive_core::id::{CertSerial, NodeId, SpiffeId};
    use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial, TrustBundle};
    use overdrive_core::wall_clock::UnixInstant;

    use super::*;

    fn material() -> SvidMaterial {
        SvidMaterial::new(
            CaCertPem::new("certificate".to_owned()),
            CaCertDer::new(vec![1, 2, 3]),
            CertSerial::new("01").expect("serial"),
            SpiffeId::new("spiffe://overdrive.local/gateway/gateway-node").expect("SPIFFE ID"),
            CaKeyPem::new("private-key".to_owned()),
            UnixInstant::from_unix_duration(Duration::from_secs(100)),
        )
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[ignore = "pending DELIVER Gateway Identity Slot"]
    fn holding_the_same_audited_identity_twice_is_idempotent() {
        let slot = Arc::new(GatewayIdentitySlot::new(
            NodeId::new("gateway-node").expect("node"),
            TrustBundle::new(CaCertPem::new("root".to_owned()), None),
        ));
        let action = slot.action_target();
        assert_eq!(
            action.hold_audited(GatewayIdentityEpoch::first(), material()),
            GatewayIdentityMutationOutcome::Applied,
        );
        let after_first = slot.current();
        assert_eq!(
            action.hold_audited(GatewayIdentityEpoch::first(), material()),
            GatewayIdentityMutationOutcome::Applied,
        );
        assert_eq!(slot.current(), after_first);
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[ignore = "pending DELIVER Gateway Identity Slot checked-epoch lifecycle"]
    fn stale_hold_and_drop_cannot_mutate_the_slot_across_disable() {
        let slot = Arc::new(GatewayIdentitySlot::new(
            NodeId::new("gateway-node").expect("node"),
            TrustBundle::new(CaCertPem::new("root".to_owned()), None),
        ));
        let action = slot.action_target();
        let lifecycle = slot.lifecycle_target();
        let future_epoch = GatewayIdentityEpoch::new(2).expect("second epoch");
        assert_eq!(
            action.hold_audited(future_epoch, material()),
            GatewayIdentityMutationOutcome::StaleEpoch,
        );
        assert!(slot.current().is_none());

        assert_eq!(
            action.hold_audited(GatewayIdentityEpoch::first(), material()),
            GatewayIdentityMutationOutcome::Applied,
        );
        let held = slot.current().expect("first epoch is held");
        assert_eq!(lifecycle.begin_disable().expect("checked next epoch"), future_epoch);
        assert_eq!(
            action.hold_audited(GatewayIdentityEpoch::first(), material()),
            GatewayIdentityMutationOutcome::StaleEpoch,
        );
        assert_eq!(
            action.drop_after_drain(GatewayIdentityEpoch::first()),
            GatewayIdentityMutationOutcome::StaleEpoch,
        );
        assert_eq!(slot.current(), Some(held), "stale effects preserve the held generation");
        assert_eq!(action.drop_after_drain(future_epoch), GatewayIdentityMutationOutcome::Applied,);
        assert!(slot.current().is_none());
    }
}
