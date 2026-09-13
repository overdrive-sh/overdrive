//! Mandatory Enabled/Disabled gateway composition held by `AppState`.
//!
//! SCAFFOLD: true. This is the exact total composition surface. Existing
//! non-gateway callers receive canonical disabled adapters; DELIVER wires the
//! enabled owners at `run_server_with_obs_and_drivers`.

use std::sync::Arc;

use overdrive_core::gateway_identity::{
    GatewayIdentityCurrentRead, GatewayIdentityDesired, GatewayIdentityDesiredRead,
    GatewayIdentityEpoch, GatewayIdentityFacts,
};
use overdrive_gateway::application::{GatewayControl, GatewayDemandComposition};
use overdrive_gateway::ports::GatewayIdentityActionTarget;
use thiserror::Error;

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
    pub const fn disabled() -> Self {
        Self { kind: GatewayIdentityActionCompositionKind::Disabled }
    }
    pub fn enabled(target: Arc<dyn GatewayIdentityActionTarget>) -> Self {
        Self { kind: GatewayIdentityActionCompositionKind::Enabled(target) }
    }
    pub(crate) fn target(
        &self,
    ) -> Result<&dyn GatewayIdentityActionTarget, GatewayIdentityDisabled> {
        match &self.kind {
            GatewayIdentityActionCompositionKind::Disabled => Err(GatewayIdentityDisabled),
            GatewayIdentityActionCompositionKind::Enabled(target) => Ok(target.as_ref()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("gateway identity action unavailable: public ingress gateway is disabled")]
pub struct GatewayIdentityDisabled;

#[derive(Clone)]
pub struct GatewayAppStateComposition {
    demand: Arc<GatewayDemandComposition>,
    identity_desired: Arc<dyn GatewayIdentityDesiredRead>,
    identity_current: Arc<dyn GatewayIdentityCurrentRead>,
    identity_actions: GatewayIdentityActionComposition,
    control: Option<GatewayControl>,
}

impl GatewayAppStateComposition {
    #[must_use]
    pub fn disabled() -> Self {
        let identity = DisabledGatewayIdentityRead::canonical();
        Self {
            demand: Arc::new(GatewayDemandComposition::disabled()),
            identity_desired: identity.clone(),
            identity_current: identity,
            identity_actions: GatewayIdentityActionComposition::disabled(),
            control: None,
        }
    }

    #[expect(dead_code, reason = "activated by the DELIVER serve composition root")]
    pub(crate) fn enabled(
        demand: Arc<GatewayDemandComposition>,
        identity_desired: Arc<dyn GatewayIdentityDesiredRead>,
        identity_current: Arc<dyn GatewayIdentityCurrentRead>,
        identity_actions: GatewayIdentityActionComposition,
        control: GatewayControl,
    ) -> Self {
        Self {
            demand,
            identity_desired,
            identity_current,
            identity_actions,
            control: Some(control),
        }
    }

    pub fn demand(&self) -> &GatewayDemandComposition {
        self.demand.as_ref()
    }
    pub fn identity_desired(&self) -> &dyn GatewayIdentityDesiredRead {
        self.identity_desired.as_ref()
    }
    pub fn identity_current(&self) -> &dyn GatewayIdentityCurrentRead {
        self.identity_current.as_ref()
    }
    pub fn identity_actions(&self) -> &GatewayIdentityActionComposition {
        &self.identity_actions
    }
    pub fn control(&self) -> Option<&GatewayControl> {
        self.control.as_ref()
    }
}

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

impl GatewayIdentityDesiredRead for DisabledGatewayIdentityRead {
    fn desired(&self) -> GatewayIdentityDesired {
        self.desired.clone()
    }
}
impl GatewayIdentityCurrentRead for DisabledGatewayIdentityRead {
    fn current(&self) -> Option<GatewayIdentityFacts> {
        None
    }
}
