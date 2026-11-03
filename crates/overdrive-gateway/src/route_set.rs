//! Serialized singleton public Route-set owner command surface.
//!
//! SCAFFOLD: true. The command bodies are DELIVER's; the DESIGN-pinned
//! shapes (`PublicRouteSetHandle::{apply,withdraw}`, the closed error enum)
//! compile today.

use overdrive_core::public_ingress::{
    Route, RouteApplyOutcome, RouteCodecError, RouteId, RouteWithdrawOutcome,
};
use overdrive_core::traits::intent_store::IntentStoreError;
use thiserror::Error;

/// Clone-safe command capability of the one Route Set owner task.
#[derive(Clone)]
pub struct PublicRouteSetHandle {
    _private: (),
}

impl PublicRouteSetHandle {
    pub async fn apply(&self, route: Route) -> Result<RouteApplyOutcome, PublicRouteSetError> {
        todo!("SCAFFOLD: PublicRouteSetHandle::apply")
    }

    pub async fn withdraw(
        &self,
        route_id: RouteId,
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
