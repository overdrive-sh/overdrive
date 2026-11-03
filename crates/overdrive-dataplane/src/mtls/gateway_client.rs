//! `HostGatewayClientMtls` — the exact-peer gateway-client mTLS adapter
//! (public-ingress-gateway, ADR-0234; the registry artifact of
//! `OUT-PIG-GATEWAY-CLIENT-EXACT-PEER`).
//!
//! On a connected fd it presents `[gateway leaf, node intermediate]` from the
//! dedicated gateway SVID slot, anchors at the trust-bundle root, requires
//! exactly one URI SAN, and requires it to equal the `SpiffeId` the dataplane
//! resolved from the receipt's `BackendId`. Only then is the kernel armed
//! through the same `HostGatewayRelay` and the leg sealed as a
//! `GatewayUpstream`; any earlier failure arms nothing and writes no body byte.
//!
//! SCAFFOLD: true. `HostGatewayClientMtls::new` and the port implementation are
//! the pinned shapes. `HostGatewayClientMtlsFactory` is NOT scaffolded: the
//! `GatewayClientMtlsFactory` / `GatewayClientMtlsDeps` pin cannot be
//! implemented as written (proposal P-20, a crate cycle).

#![allow(
    clippy::todo,
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unused_async,
    clippy::unused_self,
    dead_code,
    unused_variables,
    reason = "public-ingress DISTILL scaffold: bodies are DELIVER's"
)]

use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use overdrive_core::id::SpiffeId;
use overdrive_gateway::ports::{
    GatewayClientMtls, GatewayClientMtlsError, GatewayUpstream, GatewayUpstreamSeal,
};
use overdrive_gateway::runtime::limits::GatewayLimits;

use super::gateway_identity::GatewayClientIdentityAccess;
use crate::gateway_relay::HostGatewayRelay;

/// The Host exact-peer client. Called once by `serve` composition; supplied
/// only as `Arc<dyn GatewayClientMtls>`.
pub struct HostGatewayClientMtls {
    _private: (),
}

impl HostGatewayClientMtls {
    pub fn new(
        identity: GatewayClientIdentityAccess,
        seal: GatewayUpstreamSeal,
        limits: GatewayLimits,
        relay: Arc<HostGatewayRelay>,
    ) -> Self {
        todo!("SCAFFOLD: HostGatewayClientMtls::new")
    }
}

#[async_trait]
impl GatewayClientMtls for HostGatewayClientMtls {
    async fn authenticate(
        &self,
        connected: OwnedFd,
        expected_peer: SpiffeId,
        deadline: Instant,
    ) -> Result<GatewayUpstream, GatewayClientMtlsError> {
        todo!("SCAFFOLD: HostGatewayClientMtls::authenticate")
    }
}
