//! The mandatory Enabled/Disabled gateway composition `AppState` holds.
//!
//! One `Clone`-safe composition owns the identity read the hydration boundary
//! lends to `GatewaySvidLifecycle`, the identity action target every action
//! dispatcher consumes, and the optional `GatewayRouteCommands` and
//! `GatewayStatusReads` the Route and status handlers use. A disabled `serve`
//! holds the canonical disabled composition: an empty identity read (epoch one,
//! no subject, no current facts), a disabled action composition and no route or
//! status handle. Every existing non-gateway caller
//! receives it; only the boot composition constructs the enabled form.
//!
//! The shapes here are the DESIGN-pinned total composition surface (D5 items 6
//! and 10, "Re-pinned composition signatures"). The enabled constructor has no
//! production caller until the boot composition (steps G1-G4) lands.

use std::net::SocketAddrV4;
use std::sync::Arc;

use async_trait::async_trait;
use overdrive_core::gateway_identity::{
    GatewayIdentityDesired, GatewayIdentityEpoch, GatewayIdentityFacts, GatewayIdentityView,
};
use overdrive_core::id::{ContentHash, ServiceVip};
use overdrive_core::traits::service_vip_view::ServiceVipView;
use overdrive_gateway::application::{GatewayRouteCommands, GatewayStatusReads};
use overdrive_gateway::ports::GatewayIdentityActionTarget;

/// The identity action target the action shim's `IssueGatewaySvid` arm uses:
/// disabled (every dispatch is refused with a typed error) or enabled over the
/// one write-only slot capability.
#[derive(Clone)]
pub struct GatewayIdentityActionComposition {
    kind: GatewayIdentityActionCompositionKind,
}

#[derive(Clone)]
enum GatewayIdentityActionCompositionKind {
    Disabled,
    Enabled(Arc<dyn GatewayIdentityActionTarget>),
}

impl GatewayIdentityActionComposition {
    /// The canonical disabled composition.
    #[must_use]
    pub const fn disabled() -> Self {
        Self { kind: GatewayIdentityActionCompositionKind::Disabled }
    }

    /// The enabled composition over the slot's write-only action capability.
    #[must_use]
    pub fn enabled(target: Arc<dyn GatewayIdentityActionTarget>) -> Self {
        Self { kind: GatewayIdentityActionCompositionKind::Enabled(target) }
    }

    /// The target, or [`GatewayIdentityDisabled`] when the gateway is disabled.
    pub(crate) fn target(
        &self,
    ) -> Result<&dyn GatewayIdentityActionTarget, GatewayIdentityDisabled> {
        match &self.kind {
            GatewayIdentityActionCompositionKind::Disabled => Err(GatewayIdentityDisabled),
            GatewayIdentityActionCompositionKind::Enabled(target) => Ok(target.as_ref()),
        }
    }
}

/// A gateway identity action was dispatched while the gateway is disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("gateway identity action unavailable: public ingress gateway is disabled")]
pub struct GatewayIdentityDisabled;

/// What `AppState` carries for the gateway: identity read, identity actions and
/// the optional route-command and status-read handles.
#[derive(Clone)]
pub struct GatewayAppStateComposition {
    identity_view: Arc<dyn GatewayIdentityView>,
    identity_actions: GatewayIdentityActionComposition,
    routes: Option<GatewayRouteCommands>,
    status: Option<GatewayStatusReads>,
    configured_address: Option<SocketAddrV4>,
}

impl GatewayAppStateComposition {
    /// The canonical disabled composition: one `DisabledGatewayIdentityRead`,
    /// the disabled action composition and no route or status handle.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            identity_view: DisabledGatewayIdentityRead::canonical(),
            identity_actions: GatewayIdentityActionComposition::disabled(),
            routes: None,
            status: None,
            configured_address: None,
        }
    }

    /// The enabled composition; constructed by the boot composition only.
    pub(crate) fn enabled(
        identity_view: Arc<dyn GatewayIdentityView>,
        identity_actions: GatewayIdentityActionComposition,
        routes: GatewayRouteCommands,
        status: GatewayStatusReads,
        configured_address: SocketAddrV4,
    ) -> Self {
        Self {
            identity_view,
            identity_actions,
            routes: Some(routes),
            status: Some(status),
            configured_address: Some(configured_address),
        }
    }

    /// The configured public bind address; `None` when the gateway is disabled.
    pub(crate) const fn configured_address(&self) -> Option<SocketAddrV4> {
        self.configured_address
    }

    /// The gateway-identity read port the hydration boundary lends out.
    #[must_use]
    pub fn identity_view(&self) -> &dyn GatewayIdentityView {
        self.identity_view.as_ref()
    }

    /// The identity action composition every dispatcher consumes.
    #[must_use]
    pub const fn identity_actions(&self) -> &GatewayIdentityActionComposition {
        &self.identity_actions
    }

    /// The Route command port; `None` when the gateway is disabled.
    #[must_use]
    pub const fn routes(&self) -> Option<&GatewayRouteCommands> {
        self.routes.as_ref()
    }

    /// The redacted status read port; `None` when the gateway is disabled.
    #[must_use]
    pub const fn status(&self) -> Option<&GatewayStatusReads> {
        self.status.as_ref()
    }
}

/// The canonical disabled identity read: desired epoch one with no subject,
/// never any current facts.
#[derive(Debug)]
pub(crate) struct DisabledGatewayIdentityRead {
    desired: GatewayIdentityDesired,
}

impl DisabledGatewayIdentityRead {
    pub(crate) fn canonical() -> Arc<Self> {
        Arc::new(Self {
            desired: GatewayIdentityDesired {
                epoch: GatewayIdentityEpoch::first(),
                spiffe_id: None,
            },
        })
    }
}

impl GatewayIdentityView for DisabledGatewayIdentityRead {
    fn desired(&self) -> GatewayIdentityDesired {
        self.desired.clone()
    }

    fn current(&self) -> Option<GatewayIdentityFacts> {
        None
    }
}

/// The assigned-VIP read the gateway's Service frontend resolver uses, over the
/// one allocator `AppState` shares. The lock is taken only inside
/// [`ServiceVipView::assigned_vip`] and released before it returns, so the
/// caller holds no lock afterwards.
pub(crate) struct AllocatorServiceVipView {
    allocator:
        Arc<tokio::sync::Mutex<overdrive_dataplane::allocators::PersistentServiceVipAllocator>>,
}

impl AllocatorServiceVipView {
    pub(crate) fn new(
        allocator: Arc<
            tokio::sync::Mutex<overdrive_dataplane::allocators::PersistentServiceVipAllocator>,
        >,
    ) -> Self {
        Self { allocator }
    }
}

#[async_trait]
impl ServiceVipView for AllocatorServiceVipView {
    async fn assigned_vip(&self, spec_digest: &ContentHash) -> Option<ServiceVip> {
        self.allocator.lock().await.assigned_vip(spec_digest).await
    }
}
