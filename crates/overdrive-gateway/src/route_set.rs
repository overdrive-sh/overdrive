//! Serialized singleton public Route-set owner.
//!
//! One task owns the one store key `public-ingress/route-set`; every command
//! reads the slot, decides and writes inside that task's turn, so two
//! competing Route IDs cannot both claim the slot.

use std::path::Path;
use std::sync::Arc;

use overdrive_core::aggregate::IntentKey;
use overdrive_core::codec::envelope::EnvelopeError;
use overdrive_core::public_ingress::{
    PublicRouteSetV1, Route, RouteApplyOutcome, RouteCodecError, RouteId, RouteWithdrawOutcome,
};
use overdrive_core::traits::IntentStore;
use overdrive_core::traits::intent_store::{IntentStoreError, PutOutcome};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// Label handed to the intent codec where a store file path would go: the
/// `IntentStore` port exposes no path.
const INTENT_STORE_LABEL: &str = "<intent store>";

/// Command channel depth: commands are small and answered one at a time.
const COMMAND_CAPACITY: usize = 16;

enum RouteSetCommand {
    Apply {
        route: Route,
        reply: oneshot::Sender<Result<RouteApplyOutcome, PublicRouteSetError>>,
    },
    Withdraw {
        route_id: RouteId,
        reply: oneshot::Sender<Result<RouteWithdrawOutcome, PublicRouteSetError>>,
    },
}

/// Clone-safe command capability of the one Route Set owner task.
#[derive(Clone)]
pub struct PublicRouteSetHandle {
    commands: mpsc::Sender<RouteSetCommand>,
}

impl PublicRouteSetHandle {
    /// Declare, replace or replay the one Route.
    ///
    /// # Errors
    /// `OccupiedByDifferentRoute` when the slot holds another Route ID;
    /// `Intent` / `Decode` for store failures; `OwnerUnavailable` when the
    /// owner task is gone.
    pub async fn apply(&self, route: Route) -> Result<RouteApplyOutcome, PublicRouteSetError> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(RouteSetCommand::Apply { route, reply })
            .await
            .map_err(|_| PublicRouteSetError::OwnerUnavailable)?;
        answer.await.map_err(|_| PublicRouteSetError::OwnerUnavailable)?
    }

    /// Withdraw the Route with `route_id`.
    ///
    /// # Errors
    /// `WithdrawIdMismatch` when the slot holds another Route ID; `Intent` /
    /// `Decode` for store failures; `OwnerUnavailable` when the owner task is
    /// gone.
    pub async fn withdraw(
        &self,
        route_id: RouteId,
    ) -> Result<RouteWithdrawOutcome, PublicRouteSetError> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(RouteSetCommand::Withdraw { route_id, reply })
            .await
            .map_err(|_| PublicRouteSetError::OwnerUnavailable)?;
        answer.await.map_err(|_| PublicRouteSetError::OwnerUnavailable)?
    }
}

#[derive(Debug, Error)]
pub enum PublicRouteSetError {
    #[error("Route intent operation failed: {0}")]
    Intent(IntentStoreError),
    #[error("Route decode failed: {0}")]
    Decode(RouteCodecError),
    #[error("Route encode failed: {0}")]
    Encode(EnvelopeError),
    #[error("Route slot is occupied by {current}; attempted {attempted}")]
    OccupiedByDifferentRoute { current: RouteId, attempted: RouteId },
    #[error("Route withdrawal ID {attempted} does not match {current}")]
    WithdrawIdMismatch { current: RouteId, attempted: RouteId },
    #[error("Route owner unavailable")]
    OwnerUnavailable,
}

/// The Route Set owner: opened at builder construction (which persists the
/// canonical empty slot when absent), run as one task at start.
pub(crate) struct PublicRouteSetOwner {
    intent: Arc<dyn IntentStore>,
    commands: mpsc::Receiver<RouteSetCommand>,
}

impl PublicRouteSetOwner {
    /// Persist the canonical empty set when the key is absent and return the
    /// owner with its command handle.
    pub(crate) async fn open(
        intent: Arc<dyn IntentStore>,
    ) -> Result<(Self, PublicRouteSetHandle), PublicRouteSetError> {
        let empty = Self::encode(&PublicRouteSetV1 { route: None })?;
        match intent
            .put_if_absent(IntentKey::public_route_set().as_bytes(), empty.as_ref())
            .await
            .map_err(PublicRouteSetError::Intent)?
        {
            PutOutcome::Inserted => {}
            PutOutcome::KeyExists { .. } => {
                Self::read(intent.as_ref()).await?;
            }
        }
        let (sender, commands) = mpsc::channel(COMMAND_CAPACITY);
        Ok((Self { intent, commands }, PublicRouteSetHandle { commands: sender }))
    }

    fn encode(set: &PublicRouteSetV1) -> Result<Vec<u8>, PublicRouteSetError> {
        set.archive_for_store()
            .map(|bytes| bytes.as_slice().to_vec())
            .map_err(PublicRouteSetError::Encode)
    }

    /// The persisted slot; an absent key reads as the empty set.
    pub(crate) async fn read(
        intent: &dyn IntentStore,
    ) -> Result<Option<Route>, PublicRouteSetError> {
        let Some(bytes) = intent
            .get(IntentKey::public_route_set().as_bytes())
            .await
            .map_err(PublicRouteSetError::Intent)?
        else {
            return Ok(None);
        };
        PublicRouteSetV1::from_store_bytes(
            &bytes,
            Path::new(INTENT_STORE_LABEL),
            Some(IntentKey::public_route_set().as_str()),
        )
        .map(|set| set.route)
        .map_err(PublicRouteSetError::Decode)
    }

    async fn write(&self, route: Option<Route>) -> Result<(), PublicRouteSetError> {
        let bytes = Self::encode(&PublicRouteSetV1 { route })?;
        self.intent
            .put(IntentKey::public_route_set().as_bytes(), bytes.as_slice())
            .await
            .map_err(PublicRouteSetError::Intent)
    }

    async fn apply(&self, route: Route) -> Result<RouteApplyOutcome, PublicRouteSetError> {
        let Some(current) = Self::read(self.intent.as_ref()).await? else {
            self.write(Some(route)).await?;
            return Ok(RouteApplyOutcome::Declared);
        };
        if current.route_id() != route.route_id() {
            return Err(PublicRouteSetError::OccupiedByDifferentRoute {
                current: current.route_id().clone(),
                attempted: route.route_id().clone(),
            });
        }
        if current == route {
            return Ok(RouteApplyOutcome::Unchanged);
        }
        self.write(Some(route)).await?;
        Ok(RouteApplyOutcome::Replaced)
    }

    async fn withdraw(
        &self,
        route_id: RouteId,
    ) -> Result<RouteWithdrawOutcome, PublicRouteSetError> {
        let Some(current) = Self::read(self.intent.as_ref()).await? else {
            return Ok(RouteWithdrawOutcome::Unchanged);
        };
        if *current.route_id() != route_id {
            return Err(PublicRouteSetError::WithdrawIdMismatch {
                current: current.route_id().clone(),
                attempted: route_id,
            });
        }
        self.write(None).await?;
        Ok(RouteWithdrawOutcome::Withdrawn)
    }

    /// Serve commands until `shutdown`; a dropped reply channel is ignored.
    pub(crate) async fn run(mut self, shutdown: CancellationToken) {
        loop {
            let command = tokio::select! {
                biased;
                () = shutdown.cancelled() => return,
                command = self.commands.recv() => command,
            };
            match command {
                None => return,
                Some(RouteSetCommand::Apply { route, reply }) => {
                    let _ = reply.send(self.apply(route).await);
                }
                Some(RouteSetCommand::Withdraw { route_id, reply }) => {
                    let _ = reply.send(self.withdraw(route_id).await);
                }
            }
        }
    }
}
