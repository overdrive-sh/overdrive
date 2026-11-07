//! `HostGatewayClientMtls` — the exact-peer gateway-client mTLS adapter
//! (public-ingress-gateway, ADR-0234; the registry artifact of
//! `OUT-PIG-GATEWAY-EXACT-PEER`).
//!
//! On a connected fd it presents `[gateway leaf, node intermediate]` from the
//! dedicated gateway SVID slot, anchors at the trust-bundle root, requires
//! exactly one URI SAN, and requires it to equal the `SpiffeId` the dataplane
//! resolved from the receipt's `BackendId`. Only then is the kernel armed
//! through the same `HostGatewayRelay` and the leg sealed as a
//! `GatewayUpstream`; any earlier failure arms nothing and writes no body byte.

use std::net::TcpStream;
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use overdrive_core::id::SpiffeId;
use overdrive_gateway::ports::{
    GatewayClientIdentityAccess, GatewayClientMtls, GatewayClientMtlsDeps, GatewayClientMtlsError,
    GatewayClientMtlsFactory, GatewayUpstream, GatewayUpstreamSeal,
};
use overdrive_gateway::runtime::limits::GatewayLimits;
use rustls::pki_types::CertificateDer;

use super::tls_config;
use crate::gateway_relay::HostGatewayRelay;

/// The Host exact-peer client. Called once by `serve` composition; supplied
/// only as `Arc<dyn GatewayClientMtls>`.
pub struct HostGatewayClientMtls {
    identity: GatewayClientIdentityAccess,
    seal: GatewayUpstreamSeal,
    relay: Arc<HostGatewayRelay>,
}

impl HostGatewayClientMtls {
    /// `_limits` is part of the pinned shape; the handshake is bounded by the
    /// `deadline` each [`authenticate`](GatewayClientMtls::authenticate) call
    /// carries, so no limit is read here.
    pub const fn new(
        identity: GatewayClientIdentityAccess,
        seal: GatewayUpstreamSeal,
        _limits: GatewayLimits,
        relay: Arc<HostGatewayRelay>,
    ) -> Self {
        Self { identity, seal, relay }
    }
}

/// The peer-identity check run on the verified chain once the handshake has
/// completed and before the kernel is armed: the certificate must carry a sole
/// SPIFFE URI SAN (shape), and it must equal the selected peer (equality).
fn check_selected_peer(
    expected: &SpiffeId,
    presented: Option<&[CertificateDer<'static>]>,
) -> Result<(), GatewayClientMtlsError> {
    let leaf =
        presented.and_then(<[_]>::first).ok_or(GatewayClientMtlsError::PeerCertificateMissing)?;
    let actual =
        tls_config::sole_spiffe_uri(leaf).ok_or(GatewayClientMtlsError::PeerSpiffeShape)?;
    if &actual == expected {
        return Ok(());
    }
    Err(GatewayClientMtlsError::PeerSpiffeMismatch { expected: expected.clone(), actual })
}

#[async_trait]
impl GatewayClientMtls for HostGatewayClientMtls {
    async fn authenticate(
        &self,
        connected: OwnedFd,
        expected_peer: SpiffeId,
        deadline: Instant,
    ) -> Result<GatewayUpstream, GatewayClientMtlsError> {
        let config = self.identity.with_current(|svid, bundle| {
            tls_config::gateway_client_config(svid, bundle)
                .map_err(|_| GatewayClientMtlsError::Handshake)
        })?;
        // A descriptor that is not a connected IP socket has no selected peer address.
        let stream = TcpStream::from(connected);
        let peer = stream.peer_addr().map_err(|_| GatewayClientMtlsError::PeerAddress)?;
        let leg = self
            .relay
            .arm_after_peer_check(OwnedFd::from(stream), config, peer.ip(), deadline, |presented| {
                check_selected_peer(&expected_peer, presented)
            })
            .await?;
        Ok(self.seal.seal_authenticated(leg))
    }
}

/// Builds [`HostGatewayClientMtls`] over the one [`HostGatewayRelay`] the
/// composition shares with `GatewayAdapters.relay`.
pub struct HostGatewayClientMtlsFactory {
    relay: Arc<HostGatewayRelay>,
}

impl HostGatewayClientMtlsFactory {
    pub const fn new(relay: Arc<HostGatewayRelay>) -> Self {
        Self { relay }
    }
}

impl GatewayClientMtlsFactory for HostGatewayClientMtlsFactory {
    fn client_mtls(&self, deps: GatewayClientMtlsDeps) -> Arc<dyn GatewayClientMtls> {
        Arc::new(HostGatewayClientMtls::new(
            deps.identity,
            deps.seal,
            deps.limits,
            Arc::clone(&self.relay),
        ))
    }
}
