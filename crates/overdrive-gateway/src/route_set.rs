//! Serialized singleton public Route-set owner command surface.
//!
//! SCAFFOLD: true. The complete command bodies are acceptance-owned and
//! remain inactive until DELIVER replaces these exact channel operations.

#![expect(clippy::todo, reason = "public-ingress Route owner RED scaffold")]
#![expect(clippy::unused_async, reason = "exact Route command surface is async")]

use overdrive_core::public_ingress::{
    Route, RouteApplyOutcome, RouteCodecError, RouteId, RouteWithdrawOutcome,
};
use overdrive_core::traits::intent_store::IntentStoreError;
use thiserror::Error;

#[derive(Clone)]
pub struct PublicRouteSetHandle;

impl PublicRouteSetHandle {
    pub async fn apply(&self, _route: Route) -> Result<RouteApplyOutcome, PublicRouteSetError> {
        todo!("SCAFFOLD: PublicRouteSetHandle::apply")
    }

    pub async fn withdraw(
        &self,
        _route_id: RouteId,
    ) -> Result<RouteWithdrawOutcome, PublicRouteSetError> {
        todo!("SCAFFOLD: PublicRouteSetHandle::withdraw")
    }
}

#[derive(Debug, Error)]
pub enum PublicRouteSetError {
    #[error("Route intent operation failed: {0}")]
    Intent(IntentStoreError),
    #[error("Route decode failed: {0}")]
    Decode(RouteCodecError),
    #[error("Route slot is occupied by {current}; attempted {attempted}")]
    OccupiedByDifferentRoute { current: RouteId, attempted: RouteId },
    #[error("Route withdrawal ID {attempted} does not match {current}")]
    WithdrawIdMismatch { current: RouteId, attempted: RouteId },
    #[error("Route owner unavailable")]
    OwnerUnavailable,
}

#[expect(dead_code, reason = "activated by UnboundGatewayBuilder during DELIVER")]
pub(crate) struct PublicRouteSetOwner {
    _private: (),
}
