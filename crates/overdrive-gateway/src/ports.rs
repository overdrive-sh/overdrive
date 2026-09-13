//! Gateway-owned driven-port contracts.
//!
//! SCAFFOLD: true. Production adapters remain RED until DELIVER.

use std::os::fd::OwnedFd;
use std::pin::Pin;
use std::time::Instant;

use async_trait::async_trait;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::gateway_identity::GatewayIdentityFacts;
use overdrive_core::gateway_identity::{GatewayIdentityEpoch, GatewayIdentityEpochError};
use overdrive_core::id::{BackendId, IdParseError, SpiffeId, WorkloadId};
use overdrive_core::public_ingress::{
    GatewayConnectIntent, GatewaySelectionReceipt, ServiceListenerReference,
};
use overdrive_core::traits::ca::SvidMaterial;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::intent_store::IntentStoreError;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[async_trait]
pub trait ServiceFrontendResolve: Send + Sync {
    async fn probe(&self) -> Result<(), ServiceFrontendResolveError>;
    async fn resolve(
        &self,
        reference: &ServiceListenerReference,
    ) -> Result<ServiceFrontend, ServiceFrontendResolveError>;
}

#[derive(Debug, Error)]
pub enum ServiceFrontendResolveError {
    #[error("Service is absent: {service}")]
    ServiceAbsent { service: WorkloadId },
    #[error("Service intent read failed: {0}")]
    ServiceIntent(IntentStoreError),
    #[error("Service intent decode failed")]
    ServiceIntentDecode,
    #[error("Route target is not a Service: {service}")]
    TargetNotService { service: WorkloadId },
    #[error("listener is absent")]
    ListenerAbsent { port: std::num::NonZeroU16, protocol: overdrive_core::dataplane::Proto },
    #[error("Service VIP is unavailable")]
    ServiceVipUnavailable,
    #[error("Service frontend is invalid: {0}")]
    InvalidServiceFrontend(IdParseError),
}

#[async_trait]
pub trait GatewayConnectDataplane: Send + Sync {
    async fn probe(
        &self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Result<(), GatewayConnectDataplaneError>;
    fn register(&self, intent: GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError>;
    fn take_receipt(
        &self,
        intent: &GatewayConnectIntent,
    ) -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError>;
    fn cleanup(&self, intent: &GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError>;
    fn cleanup_all_gateway_intents(
        &self,
    ) -> Result<GatewayConnectCleanupSweep, GatewayConnectDataplaneError>;
    fn selected_backend_identity(
        &self,
        backend_id: BackendId,
    ) -> Result<SpiffeId, GatewayConnectDataplaneError>;
    fn live_intent_count(&self) -> Result<u32, GatewayConnectDataplaneError>;
    fn live_receipt_count(&self) -> Result<u32, GatewayConnectDataplaneError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayConnectCleanupSweep {
    pub removed_intents: u32,
    pub removed_receipts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GatewayConnectDataplaneError {
    #[error("gateway connect dataplane unavailable")]
    Unavailable,
    #[error("gateway connect dataplane probe timed out")]
    ProbeTimeout,
    #[error("gateway intent registry is full")]
    IntentRegistryFull,
    #[error("gateway intent is already registered")]
    IntentAlreadyRegistered,
    #[error("gateway intent is missing")]
    IntentMissing,
    #[error("gateway receipt is missing")]
    ReceiptMissing,
    #[error("gateway receipt does not match its intent")]
    ReceiptMismatch,
    #[error("selected backend identity is missing for {backend_id}")]
    BackendIdentityMissing { backend_id: BackendId },
    #[error("gateway kernel operation failed")]
    Kernel,
    #[error("gateway connect cleanup failed")]
    Cleanup,
}

#[async_trait]
pub trait GatewayIdentityLifecycleControl: Send + Sync {
    async fn ensure_current(
        &self,
        deadline: Instant,
    ) -> Result<GatewayIdentityFacts, GatewayIdentityLifecycleError>;
    fn current(&self) -> Option<GatewayIdentityFacts>;
    fn subscribe(&self) -> tokio::sync::watch::Receiver<Option<GatewayIdentityFacts>>;
    async fn disable_after_drain(
        &self,
        deadline: Instant,
    ) -> Result<(), GatewayIdentityLifecycleError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayIdentityWaitPhase {
    EnsureCurrent,
    DisableAfterDrain,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GatewayIdentityLifecycleError {
    #[error("gateway identity reconciler unavailable")]
    ReconcilerUnavailable,
    #[error("gateway identity epoch exhausted")]
    EpochExhausted,
    #[error("gateway SVID issuance failed")]
    IssueFailed,
    #[error("gateway SVID drop failed")]
    DropFailed,
    #[error("gateway identity wait timed out during {phase:?}")]
    Timeout { phase: GatewayIdentityWaitPhase },
    #[error("gateway identity owner closed")]
    Closed,
}

pub trait GatewayIdentityLifecycleTarget: Send + Sync {
    fn current(&self) -> Option<GatewayIdentityFacts>;
    fn subscribe(&self) -> tokio::sync::watch::Receiver<Option<GatewayIdentityFacts>>;
    fn begin_disable(&self) -> Result<GatewayIdentityEpoch, GatewayIdentityEpochError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayIdentityMutationOutcome {
    Applied,
    StaleEpoch,
}

pub trait GatewayIdentityActionTarget: Send + Sync {
    fn hold_audited(
        &self,
        epoch: GatewayIdentityEpoch,
        material: SvidMaterial,
    ) -> GatewayIdentityMutationOutcome;
    fn drop_after_drain(&self, epoch: GatewayIdentityEpoch) -> GatewayIdentityMutationOutcome;
}

pub trait GatewayByteStream: AsyncRead + AsyncWrite + Unpin + Send + 'static {}
impl<T> GatewayByteStream for T where T: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

pub struct GatewayUpstreamSeal {
    _private: (),
}

pub struct GatewayUpstream {
    io: Pin<Box<dyn GatewayByteStream>>,
}

impl GatewayUpstreamSeal {
    pub fn seal_authenticated(&self, _io: Pin<Box<dyn GatewayByteStream>>) -> GatewayUpstream {
        todo!("SCAFFOLD: GatewayUpstreamSeal::seal_authenticated")
    }
}

impl AsyncRead for GatewayUpstream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.io.as_mut().poll_read(cx, buf)
    }
}

impl AsyncWrite for GatewayUpstream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.io.as_mut().poll_write(cx, buf)
    }
    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.io.as_mut().poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.io.as_mut().poll_shutdown(cx)
    }
}

#[async_trait]
pub trait GatewayClientMtls: Send + Sync {
    async fn authenticate(
        &self,
        connected: OwnedFd,
        expected_peer: SpiffeId,
        deadline: Instant,
    ) -> Result<GatewayUpstream, GatewayClientMtlsError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GatewayClientMtlsError {
    #[error("gateway identity is absent")]
    IdentityAbsent,
    #[error("gateway identity is expired")]
    IdentityExpired,
    #[error("selected peer address is unavailable")]
    PeerAddress,
    #[error("gateway mTLS handshake timed out")]
    HandshakeTimeout,
    #[error("gateway mTLS handshake failed")]
    Handshake,
    #[error("peer certificate is missing")]
    PeerCertificateMissing,
    #[error("peer SPIFFE identity shape is invalid")]
    PeerSpiffeShape,
    #[error("peer SPIFFE identity mismatch")]
    PeerSpiffeMismatch { expected: SpiffeId, actual: SpiffeId },
}
