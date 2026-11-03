//! Gateway-owned driven-port contracts.
//!
//! SCAFFOLD: true. Every shape is DESIGN-pinned (Gateway upstream path,
//! Kernel relay data path (G1), Gateway identity ports). Adapters live in
//! `overdrive-dataplane` (host) and `overdrive-sim` (Sim); behaviour bodies
//! that this crate owns fail with SCAFFOLD text until DELIVER.

use std::any::Any;
use std::num::{NonZeroU16, NonZeroU64, NonZeroUsize};
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_core::dataplane::{Proto, ServiceFrontend};
use overdrive_core::gateway_identity::{
    GatewayIdentityEpoch, GatewayIdentityEpochError, GatewayIdentityFacts,
};
use overdrive_core::id::{BackendId, IdParseError, SpiffeId, WorkloadId};
use overdrive_core::public_ingress::{
    PublicHostname, ServiceKey, ServiceListenerReference, SocketCookie,
};
use overdrive_core::traits::ca::SvidMaterial;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::intent_store::IntentStoreError;
use thiserror::Error;
use tokio::sync::watch;

// ---------------------------------------------------------------------------
// Service frontend resolution
// ---------------------------------------------------------------------------

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
    ListenerAbsent { port: NonZeroU16, protocol: Proto },
    #[error("Service VIP is unavailable")]
    ServiceVipUnavailable,
    #[error("Service frontend is invalid: {0}")]
    InvalidServiceFrontend(IdParseError),
}

// ---------------------------------------------------------------------------
// Gateway connect dataplane (ADR-0243)
// ---------------------------------------------------------------------------

#[async_trait]
pub trait GatewayConnectDataplane: Send + Sync {
    /// First step of the boot gate: sweep residue, then run the sentinel
    /// `192.0.2.1:9/tcp` connect.
    async fn probe(
        &self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Result<(), GatewayConnectDataplaneError>;
    /// Marks, loopback-binds, reads the cookie of and registers one new
    /// upstream socket. The only constructor of a registered intent.
    fn open_upstream(
        &self,
        key: ServiceKey,
    ) -> Result<PreparedUpstream, GatewayConnectDataplaneError>;
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

/// The registered intent of one prepared upstream socket.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GatewayConnectIntent {
    pub socket_cookie: SocketCookie,
    pub service_key: ServiceKey,
}

/// The hook's selection receipt for one registered intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewaySelectionReceipt {
    Selected { socket_cookie: SocketCookie, service_key: ServiceKey, backend_id: BackendId },
    NoBackend { socket_cookie: SocketCookie, service_key: ServiceKey },
}

/// A socket prepared by the dataplane adapter. Not `Clone`; carries no
/// cleanup guard (Cross-crate construction of `PreparedUpstream`, DG-8).
pub struct PreparedUpstream {
    socket: tokio::net::TcpSocket,
    intent: GatewayConnectIntent,
}

impl PreparedUpstream {
    /// Backends only. Registers nothing and validates nothing.
    pub fn new(socket: tokio::net::TcpSocket, intent: GatewayConnectIntent) -> Self {
        Self { socket, intent }
    }
    pub fn intent(&self) -> &GatewayConnectIntent {
        &self.intent
    }
    /// Hands the socket to the caller for `connect(2)`; the intent stays
    /// registered until `cleanup(&intent)`.
    pub fn into_parts(self) -> (tokio::net::TcpSocket, GatewayConnectIntent) {
        (self.socket, self.intent)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GatewayConnectDataplaneError {
    #[error("gateway connect dataplane unavailable")]
    Unavailable,
    #[error("gateway connect dataplane probe timed out")]
    ProbeTimeout,
    #[error("gateway upstream socket creation failed: {0:?}")]
    SocketCreate(std::io::ErrorKind),
    #[error("gateway upstream socket option failed: {0:?}")]
    SocketOption(std::io::ErrorKind),
    #[error("gateway upstream socket bind failed: {0:?}")]
    SocketBind(std::io::ErrorKind),
    #[error("gateway upstream socket cookie read failed: {0:?}")]
    CookieRead(std::io::ErrorKind),
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

// ---------------------------------------------------------------------------
// Gateway identity lifecycle (ADR-0225; disable is a synchronous clear, R-4)
// ---------------------------------------------------------------------------

#[async_trait]
pub trait GatewayIdentityLifecycleControl: Send + Sync {
    async fn ensure_current(
        &self,
        deadline: Instant,
    ) -> Result<GatewayIdentityFacts, GatewayIdentityLifecycleError>;
    fn current(&self) -> Option<GatewayIdentityFacts>;
    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>>;
    /// Synchronous clear: bumps the epoch and empties the slot in the same
    /// call. `deadline` is not read by the clear.
    async fn disable_after_drain(
        &self,
        deadline: Instant,
    ) -> Result<(), GatewayIdentityLifecycleError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayIdentityWaitPhase {
    EnsureCurrent,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GatewayIdentityLifecycleError {
    #[error("gateway identity reconciler unavailable")]
    ReconcilerUnavailable,
    #[error("gateway identity epoch exhausted")]
    EpochExhausted,
    #[error("gateway SVID issuance failed")]
    IssueFailed,
    #[error("gateway identity wait timed out during {phase:?}")]
    Timeout { phase: GatewayIdentityWaitPhase },
    #[error("gateway identity owner closed")]
    Closed,
}

pub trait GatewayIdentityLifecycleTarget: Send + Sync {
    fn current(&self) -> Option<GatewayIdentityFacts>;
    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>>;
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
}

// ---------------------------------------------------------------------------
// Gateway-client mTLS and the sealed authenticated upstream
// ---------------------------------------------------------------------------

/// The proof that the peer was authenticated and the leg armed. No public
/// constructor, no `Clone`, no conversion except `into_halves`.
pub struct GatewayUpstreamSeal {
    _private: (),
}

pub struct GatewayUpstream {
    leg: KtlsLeg,
}

impl GatewayUpstreamSeal {
    pub fn seal_authenticated(&self, leg: KtlsLeg) -> GatewayUpstream {
        todo!("SCAFFOLD: GatewayUpstreamSeal::seal_authenticated")
    }
}

impl GatewayUpstream {
    pub fn into_halves(self) -> (KtlsReadHalf, KtlsWriteHalf) {
        self.leg.into_halves()
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
    /// Raised after the peer equality held: the kernel refused to arm TLS.
    #[error("kernel TLS arming failed: {0:?}")]
    Arm(KtlsArmError),
}

// ---------------------------------------------------------------------------
// Kernel-TLS relay (G1): opaque legs, deadlines and the driven port
// ---------------------------------------------------------------------------

/// Opaque, non-Clone kernel-TLS leg: a TCP socket on which TLS 1.3 TX and RX
/// are armed. Built only by the backend that owns the leg.
pub struct KtlsLeg {
    read: Box<dyn Any + Send>,
    write: Box<dyn Any + Send>,
}

impl KtlsLeg {
    /// The one constructor. The caller guarantees TLS 1.3 TX and RX are armed.
    pub fn from_armed(read: Box<dyn Any + Send>, write: Box<dyn Any + Send>) -> Self {
        todo!("SCAFFOLD: KtlsLeg::from_armed")
    }
    pub fn into_halves(self) -> (KtlsReadHalf, KtlsWriteHalf) {
        todo!("SCAFFOLD: KtlsLeg::into_halves")
    }
}

/// Receive half. Owns at most one private pipe (ADR-0246).
pub struct KtlsReadHalf {
    backend: Box<dyn Any + Send>,
}

/// Transmit half. Shares the leg's transmit lock with the read half.
pub struct KtlsWriteHalf {
    backend: Box<dyn Any + Send>,
}

impl KtlsReadHalf {
    /// `unreachable!()` on a `T` the backend did not store.
    pub fn backend_mut<T: Any>(&mut self) -> &mut T {
        todo!("SCAFFOLD: KtlsReadHalf::backend_mut")
    }
}

impl KtlsWriteHalf {
    /// `unreachable!()` on a `T` the backend did not store.
    pub fn backend_mut<T: Any>(&mut self) -> &mut T {
        todo!("SCAFFOLD: KtlsWriteHalf::backend_mut")
    }
}

pub struct AcceptedPublicLeg {
    pub leg: KtlsLeg,
    pub sni: PublicHostname,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadTerminator {
    HeaderBlockEnd,
    LineEnd,
}

/// Application octets through and including the terminator.
pub struct ThroughBytes(Vec<u8>);

impl ThroughBytes {
    pub fn new(bytes: Vec<u8>) -> Self {
        todo!("SCAFFOLD: ThroughBytes::new")
    }
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RelayDeadlines {
    pub absolute: Instant,
    pub no_progress: Duration,
}

/// Deadlines of one head or framing-line read.
///
/// `first_octet`: absolute deadline for the first octet of the item;
/// `complete_within`: budget once the first octet arrived; `complete_by`:
/// absolute ceiling on the whole item. The port applies the earliest.
#[derive(Debug, Clone, Copy)]
pub struct ReadDeadlines {
    pub first_octet: Instant,
    pub complete_within: Option<Duration>,
    pub complete_by: Option<Instant>,
}

#[async_trait]
pub trait GatewayRelay: Send + Sync {
    /// Earned-Trust probe, called once per boot in gate G-1.
    async fn probe(&self, deadline: Instant) -> Result<(), RelayProbeError>;
    /// Completes the TLS 1.3 server handshake with `tls` and arms kTLS TX+RX.
    async fn accept_public(
        &self,
        accepted: OwnedFd,
        tls: Arc<rustls::ServerConfig>,
        deadline: Instant,
    ) -> Result<AcceptedPublicLeg, PublicHandshakeError>;
    /// Consumes application octets through the first terminator, no more.
    async fn read_through(
        &self,
        from: &mut KtlsReadHalf,
        terminator: HeadTerminator,
        limit: NonZeroUsize,
        deadlines: ReadDeadlines,
    ) -> Result<ThroughBytes, LegReadError>;
    /// Writes every octet or fails; one writer at a time per half.
    async fn write_all(
        &self,
        to: &mut KtlsWriteHalf,
        bytes: &[u8],
        deadline: Instant,
    ) -> Result<(), LegWriteError>;
    /// Moves exactly `len` application octets from `from` to `to` by splice.
    async fn relay_exact(
        &self,
        from: &mut KtlsReadHalf,
        to: &mut KtlsWriteHalf,
        len: NonZeroU64,
        deadlines: RelayDeadlines,
    ) -> Result<(), RelayError>;
    /// Moves octets until the source's close_notify, failing `LimitExceeded`
    /// after more than `max`. Returns the count (R-12).
    async fn relay_to_end(
        &self,
        from: &mut KtlsReadHalf,
        to: &mut KtlsWriteHalf,
        max: NonZeroU64,
        deadlines: RelayDeadlines,
    ) -> Result<u64, RelayError>;
    /// Sends close_notify, then shuts down the transmit direction.
    async fn close_write(
        &self,
        to: &mut KtlsWriteHalf,
        deadline: Instant,
    ) -> Result<(), LegWriteError>;
    /// Reads and drops application octets until the peer's close_notify,
    /// `max` octets or `deadlines` (R-14). Returns the octets dropped.
    async fn discard_remaining(
        &self,
        from: &mut KtlsReadHalf,
        max: NonZeroU64,
        deadlines: RelayDeadlines,
    ) -> Result<u64, LegReadError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RelayProbeError {
    #[error("kernel TLS arming is unavailable: {0:?}")]
    Arm(KtlsArmError),
    #[error("relay pipe is unusable: {0:?}")]
    Pipe(RelayPipeError),
    #[error("splice moved {moved} of {expected} octets")]
    SpliceExactness { moved: u64, expected: u64 },
    #[error("relay probe timed out")]
    Timeout,
    #[error("relay probe I/O failed: {kind:?}")]
    Io { kind: std::io::ErrorKind },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PublicHandshakeError {
    #[error("public TLS handshake timed out")]
    Timeout,
    #[error("public TLS handshake failed")]
    Tls { alert: Option<u8> },
    #[error("client did not offer ALPN http/1.1")]
    AlpnMismatch,
    #[error("client sent no server name")]
    ServerNameMissing,
    #[error("kernel TLS arming failed: {0:?}")]
    Arm(KtlsArmError),
    #[error("public handshake I/O failed: {kind:?}")]
    Io { kind: std::io::ErrorKind },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KtlsArmError {
    UlpInstall { errno: i32 },
    CryptoInfo { errno: i32 },
    CipherNotSupported,
    KernelConnection,
}

/// A peer-initiated KeyUpdate that the relay failed to answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyUpdateError {
    RustlsRefused,
    RearmRx { errno: i32 },
    RearmTx { errno: i32 },
    Io { kind: std::io::ErrorKind },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LegReadError {
    #[error("peer sent close_notify")]
    CleanEof,
    #[error("leg ended before the item completed")]
    Truncated,
    #[error("peer alert {description}")]
    Alert { description: u8 },
    #[error("unexpected TLS record type {content_type}")]
    UnexpectedRecord { content_type: u8 },
    #[error("KeyUpdate failed: {0:?}")]
    KeyUpdate(KeyUpdateError),
    #[error("no terminator within {limit} octets")]
    LimitExceeded { limit: usize },
    #[error("read deadline elapsed")]
    Deadline,
    #[error("leg read I/O failed: {kind:?}")]
    Io { kind: std::io::ErrorKind },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LegWriteError {
    #[error("peer closed the leg")]
    PeerClosed,
    #[error("peer alert {description}")]
    Alert { description: u8 },
    #[error("KeyUpdate failed: {0:?}")]
    KeyUpdate(KeyUpdateError),
    #[error("write deadline elapsed")]
    Deadline,
    #[error("leg write I/O failed: {kind:?}")]
    Io { kind: std::io::ErrorKind },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RelayError {
    #[error("relay read failed: {0}")]
    Read(LegReadError),
    #[error("relay write failed: {0}")]
    Write(LegWriteError),
    #[error("source ended with {remaining} octets owed")]
    SourceEndedMidBody { remaining: u64 },
    #[error("relay exceeded {max} octets")]
    LimitExceeded { max: u64 },
    #[error("relay made no progress")]
    NoProgress,
    #[error("relay deadline elapsed")]
    Deadline,
    #[error("relay pipe failed: {0}")]
    Pipe(RelayPipeError),
}

/// The pipe floor is a typestate fact (ADR-0246); the only runtime outcomes
/// are a failed creation or a capacity that reads back below the floor.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RelayPipeError {
    #[error("relay pipe creation failed: {kind:?}")]
    Create { kind: std::io::ErrorKind },
    #[error("relay pipe capacity {got} is below the floor")]
    CapacityBelowFloor { got: usize },
}
