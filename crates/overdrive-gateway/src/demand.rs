//! Active Gateway Frontend Demand owner command surface.
//!
//! SCAFFOLD: true. DELIVER replaces the marked owner-channel operations.

#![expect(clippy::todo, reason = "public-ingress demand-owner RED scaffold")]
#![expect(clippy::unused_async, reason = "exact demand command surface is async")]
#![allow(dead_code, reason = "activated by the DELIVER Gateway Application owner")]

use std::time::Instant;

use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::public_ingress::{
    GatewayApplicationGenerationId, GatewayFrontendDemandError, GatewayFrontendDemandRevision,
    ServiceKey,
};
use overdrive_core::traits::clock::Clock;

pub(crate) struct GatewayFrontendDemandTicket {
    generation: GatewayApplicationGenerationId,
    revision: GatewayFrontendDemandRevision,
    service_key: ServiceKey,
}

pub(crate) struct GatewayFrontendDemandHandle {
    _private: (),
}

impl GatewayFrontendDemandHandle {
    pub(crate) async fn stage(
        &self,
        _generation: GatewayApplicationGenerationId,
        _frontend: ServiceFrontend,
    ) -> Result<GatewayFrontendDemandTicket, GatewayFrontendDemandError> {
        todo!("SCAFFOLD: GatewayFrontendDemandHandle::stage")
    }
    pub(crate) async fn wait_applied(
        &self,
        _ticket: &GatewayFrontendDemandTicket,
        _deadline: Instant,
        _clock: &dyn Clock,
    ) -> Result<(), GatewayFrontendDemandError> {
        todo!("SCAFFOLD: GatewayFrontendDemandHandle::wait_applied")
    }
    pub(crate) async fn promote(
        &self,
        _ticket: GatewayFrontendDemandTicket,
    ) -> Result<(), GatewayFrontendDemandError> {
        todo!("SCAFFOLD: GatewayFrontendDemandHandle::promote")
    }
    pub(crate) async fn remove_staged(
        &self,
        _generation: GatewayApplicationGenerationId,
        _deadline: Instant,
        _clock: &dyn Clock,
    ) -> Result<(), GatewayFrontendDemandError> {
        todo!("SCAFFOLD: GatewayFrontendDemandHandle::remove_staged")
    }
    pub(crate) async fn begin_draining(
        &self,
        _generation: GatewayApplicationGenerationId,
    ) -> Result<(), GatewayFrontendDemandError> {
        todo!("SCAFFOLD: GatewayFrontendDemandHandle::begin_draining")
    }
    pub(crate) async fn retire(
        &self,
        _generation: GatewayApplicationGenerationId,
    ) -> Result<(), GatewayFrontendDemandError> {
        todo!("SCAFFOLD: GatewayFrontendDemandHandle::retire")
    }
    pub(crate) async fn clear_all(
        &self,
        _deadline: Instant,
        _clock: &dyn Clock,
    ) -> Result<(), GatewayFrontendDemandError> {
        todo!("SCAFFOLD: GatewayFrontendDemandHandle::clear_all")
    }
}
