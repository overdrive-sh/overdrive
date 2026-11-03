//! The dedicated gateway SVID slot (public-ingress-gateway, ADR-0225).
//!
//! `GatewayIdentitySlot` holds exactly one non-allocation SVID generation for
//! the node's gateway subject, under the existing dataplane mTLS owner. It
//! never enters `IdentityMgr` or the generic `IdentityRead`. Its lock also owns
//! a checked process-local desired epoch: the constructor's epoch one means
//! enabled; `begin_disable` increments the epoch and empties the slot in the
//! same call (R-4: disable is a synchronous clear); `hold_audited` rejects an
//! action carrying a stale epoch, so an in-flight issue cannot re-hold after a
//! disable.
//!
//! SCAFFOLD: true. Only the shapes the DESIGN pins ("Exact identity,
//! connector and authenticated-stream ports") are present, with
//! `GatewayClientIdentityAccess` where the DESIGN currently pins it (this
//! module). Its placement relative to `GatewayClientMtlsDeps` is the open
//! proposal P-20 (a crate cycle); the factory and deps types are not
//! scaffolded anywhere.

#![allow(
    clippy::todo,
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unused_self,
    dead_code,
    unused_variables,
    reason = "public-ingress DISTILL scaffold: bodies are DELIVER's"
)]

use std::sync::Arc;

use overdrive_core::gateway_identity::{
    GatewayIdentityDesired, GatewayIdentityEpoch, GatewayIdentityEpochError, GatewayIdentityFacts,
    GatewayIdentityView,
};
use overdrive_core::id::NodeId;
use overdrive_core::traits::ca::{SvidMaterial, TrustBundle};
use overdrive_core::traits::clock::Clock;
use overdrive_gateway::ports::{
    GatewayClientMtlsError, GatewayIdentityActionTarget, GatewayIdentityLifecycleTarget,
    GatewayIdentityMutationOutcome,
};
use tokio::sync::watch;

/// The single gateway SVID holder.
pub struct GatewayIdentitySlot {
    _private: (),
}

impl GatewayIdentitySlot {
    /// Epoch one, enabled, empty.
    pub fn new(node_id: NodeId, trust_bundle: TrustBundle) -> Self {
        todo!("SCAFFOLD: GatewayIdentitySlot::new")
    }

    /// The checked desired epoch and subject.
    pub fn desired(&self) -> GatewayIdentityDesired {
        todo!("SCAFFOLD: GatewayIdentitySlot::desired")
    }

    /// Non-secret facts of the held SVID, `None` while empty.
    pub fn current(&self) -> Option<GatewayIdentityFacts> {
        todo!("SCAFFOLD: GatewayIdentitySlot::current")
    }

    /// Currentness stream of the non-secret facts.
    pub fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>> {
        todo!("SCAFFOLD: GatewayIdentitySlot::subscribe")
    }

    /// The capability the exact-peer client borrows held material through.
    pub fn client_access(self: &Arc<Self>, clock: Arc<dyn Clock>) -> GatewayClientIdentityAccess {
        todo!("SCAFFOLD: GatewayIdentitySlot::client_access")
    }

    /// The write-only action capability installed in the application state.
    pub fn action_target(self: &Arc<Self>) -> GatewayIdentityActionHandle {
        todo!("SCAFFOLD: GatewayIdentitySlot::action_target")
    }

    /// The lifecycle capability the control-plane adapter calls to disable.
    pub fn lifecycle_target(self: &Arc<Self>) -> GatewayIdentityLifecycleHandle {
        todo!("SCAFFOLD: GatewayIdentitySlot::lifecycle_target")
    }

    /// Bump the epoch and empty the slot in the same call.
    pub(crate) fn begin_disable(&self) -> Result<GatewayIdentityEpoch, GatewayIdentityEpochError> {
        todo!("SCAFFOLD: GatewayIdentitySlot::begin_disable")
    }

    /// Hold `material` for `epoch` after the audit row is durable; a stale
    /// epoch is a successful no-op.
    pub(crate) fn hold_audited(
        &self,
        epoch: GatewayIdentityEpoch,
        material: SvidMaterial,
    ) -> GatewayIdentityMutationOutcome {
        todo!("SCAFFOLD: GatewayIdentitySlot::hold_audited")
    }
}

impl GatewayIdentityView for GatewayIdentitySlot {
    fn desired(&self) -> GatewayIdentityDesired {
        todo!("SCAFFOLD: GatewayIdentityView::desired")
    }

    fn current(&self) -> Option<GatewayIdentityFacts> {
        todo!("SCAFFOLD: GatewayIdentityView::current")
    }
}

/// Write-only capability: the only implementer of `GatewayIdentityActionTarget`.
pub struct GatewayIdentityActionHandle {
    slot: Arc<GatewayIdentitySlot>,
}

/// Lifecycle capability: the only implementer of `GatewayIdentityLifecycleTarget`.
pub struct GatewayIdentityLifecycleHandle {
    slot: Arc<GatewayIdentitySlot>,
}

impl GatewayIdentityActionTarget for GatewayIdentityActionHandle {
    fn hold_audited(
        &self,
        epoch: GatewayIdentityEpoch,
        material: SvidMaterial,
    ) -> GatewayIdentityMutationOutcome {
        todo!("SCAFFOLD: GatewayIdentityActionHandle::hold_audited")
    }
}

impl GatewayIdentityLifecycleTarget for GatewayIdentityLifecycleHandle {
    fn current(&self) -> Option<GatewayIdentityFacts> {
        todo!("SCAFFOLD: GatewayIdentityLifecycleHandle::current")
    }

    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>> {
        todo!("SCAFFOLD: GatewayIdentityLifecycleHandle::subscribe")
    }

    fn begin_disable(&self) -> Result<GatewayIdentityEpoch, GatewayIdentityEpochError> {
        todo!("SCAFFOLD: GatewayIdentityLifecycleHandle::begin_disable")
    }
}

/// Borrowing capability for the exact-peer client.
///
/// Runs a non-async closure while the slot keeps the held SVID borrowed. It
/// reads the injected clock, rejects absent or expired material, and cannot
/// return or clone the SVID, key or trust bundle.
pub struct GatewayClientIdentityAccess {
    slot: Arc<GatewayIdentitySlot>,
    clock: Arc<dyn Clock>,
}

impl GatewayClientIdentityAccess {
    /// `IdentityAbsent` / `IdentityExpired` when no time-usable material is held.
    pub(crate) fn with_current<R>(
        &self,
        use_material: impl FnOnce(&SvidMaterial, &TrustBundle) -> Result<R, GatewayClientMtlsError>,
    ) -> Result<R, GatewayClientMtlsError> {
        todo!("SCAFFOLD: GatewayClientIdentityAccess::with_current")
    }
}
