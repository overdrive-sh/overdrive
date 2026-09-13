//! Active Gateway Application owner and composition surface.
//!
//! SCAFFOLD: true. Every owner method intentionally remains RED until DELIVER.

#![expect(clippy::unused_async, reason = "exact async RED scaffold signatures")]
#![allow(clippy::doc_markdown, reason = "exact Contract Shape test metadata")]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use overdrive_core::public_ingress::{
    GatewayApplicationStatusRowV1, GatewayConnectIntent, GatewayFrontendDemandAcknowledge,
    GatewayFrontendDemandError, GatewayFrontendDemandRead, GatewayFrontendDemandRevision,
    GatewayFrontendDemandSnapshot, GatewayFrontendDemandWake, PublicCertifiedKeyStatusRowV1,
};
use overdrive_core::traits::observation_store::ObservationStoreError;
use thiserror::Error;

use crate::ports::{GatewayClientMtls, GatewayConnectDataplane, GatewayIdentityLifecycleControl};
use crate::route_set::{PublicRouteSetError, PublicRouteSetHandle};
use crate::runtime::GatewayLimits;

pub struct GatewayRuntimeDependencies;
impl GatewayRuntimeDependencies {
    pub fn new(
        _limits: GatewayLimits,
        _dataplane: Arc<dyn GatewayConnectDataplane>,
        _client_mtls: Arc<dyn GatewayClientMtls>,
        _identity_lifecycle: Arc<dyn GatewayIdentityLifecycleControl>,
    ) -> Self {
        todo!("SCAFFOLD: GatewayRuntimeDependencies::new")
    }
}

#[derive(Clone)]
pub struct GatewayControl;
impl GatewayControl {
    pub fn routes(&self) -> &PublicRouteSetHandle {
        todo!("SCAFFOLD: GatewayControl::routes")
    }
    pub async fn application_status(
        &self,
    ) -> Result<Option<GatewayApplicationStatusRowV1>, GatewayStatusReadError> {
        todo!("SCAFFOLD: GatewayControl::application_status")
    }
    pub async fn certified_key_status(
        &self,
    ) -> Result<Option<PublicCertifiedKeyStatusRowV1>, GatewayStatusReadError> {
        todo!("SCAFFOLD: GatewayControl::certified_key_status")
    }
}

#[derive(Debug, Error)]
pub enum GatewayStatusReadError {
    #[error("Gateway Application status read failed: {0}")]
    Application(ObservationStoreError),
    #[error("public certified-key status read failed: {0}")]
    CertifiedKey(ObservationStoreError),
}

pub struct GatewayDemandComposition {
    kind: GatewayDemandCompositionKind,
}

#[cfg_attr(test, allow(dead_code, reason = "Enabled has no live unit-test constructor"))]
enum GatewayDemandCompositionKind {
    Disabled {
        read: Arc<dyn GatewayFrontendDemandRead>,
    },
    Enabled {
        read: Arc<dyn GatewayFrontendDemandRead>,
        acknowledge: Arc<dyn GatewayFrontendDemandAcknowledge>,
        wake: Arc<dyn GatewayFrontendDemandWake>,
    },
}

impl GatewayDemandComposition {
    pub fn disabled() -> Self {
        Self {
            kind: GatewayDemandCompositionKind::Disabled {
                read: Arc::new(EmptyGatewayFrontendDemand),
            },
        }
    }
    #[cfg_attr(not(test), expect(dead_code, reason = "activated by the DELIVER composition root"))]
    #[cfg_attr(test, allow(dead_code, reason = "enabled composition is exercised cross-crate"))]
    pub(crate) fn enabled(
        read: Arc<dyn GatewayFrontendDemandRead>,
        acknowledge: Arc<dyn GatewayFrontendDemandAcknowledge>,
        wake: Arc<dyn GatewayFrontendDemandWake>,
    ) -> Self {
        Self { kind: GatewayDemandCompositionKind::Enabled { read, acknowledge, wake } }
    }
    pub fn read(&self) -> &dyn GatewayFrontendDemandRead {
        match &self.kind {
            GatewayDemandCompositionKind::Disabled { read }
            | GatewayDemandCompositionKind::Enabled { read, .. } => read.as_ref(),
        }
    }
    pub fn dispatch_ports(&self) -> Option<GatewayDemandDispatchPorts> {
        match &self.kind {
            GatewayDemandCompositionKind::Disabled { .. } => None,
            GatewayDemandCompositionKind::Enabled { read, acknowledge, wake } => {
                Some(GatewayDemandDispatchPorts {
                    read: Arc::clone(read),
                    acknowledge: Arc::clone(acknowledge),
                    wake: Arc::clone(wake),
                })
            }
        }
    }
}

impl GatewayFrontendDemandRead for GatewayDemandComposition {
    fn snapshot(&self) -> Arc<GatewayFrontendDemandSnapshot> {
        self.read().snapshot()
    }
}

#[derive(Clone)]
pub struct GatewayDemandDispatchPorts {
    read: Arc<dyn GatewayFrontendDemandRead>,
    acknowledge: Arc<dyn GatewayFrontendDemandAcknowledge>,
    wake: Arc<dyn GatewayFrontendDemandWake>,
}

impl GatewayDemandDispatchPorts {
    pub fn read(&self) -> &dyn GatewayFrontendDemandRead {
        self.read.as_ref()
    }
    pub fn acknowledge(&self) -> &dyn GatewayFrontendDemandAcknowledge {
        self.acknowledge.as_ref()
    }
    pub fn wake(&self) -> &dyn GatewayFrontendDemandWake {
        self.wake.as_ref()
    }
}

#[derive(Debug, Default)]
pub struct EmptyGatewayFrontendDemand;
impl GatewayFrontendDemandRead for EmptyGatewayFrontendDemand {
    fn snapshot(&self) -> Arc<GatewayFrontendDemandSnapshot> {
        Arc::new(GatewayFrontendDemandSnapshot {
            revision: GatewayFrontendDemandRevision::first(),
            entries: Vec::new(),
        })
    }
}

pub struct GatewayBuilder;
pub struct UnboundGatewayBuilder;
pub struct StartedGateway;
pub struct GatewayHandle;

pub(crate) struct GatewayAdmissionBudget;
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "DISTILL RED scaffold is activated by connector ownership")
)]
impl GatewayAdmissionBudget {
    pub(crate) fn new(_limits: Arc<GatewayLimits>) -> Arc<Self> {
        todo!("SCAFFOLD: GatewayAdmissionBudget::new")
    }
}

pub(crate) struct GatewayCleanupLedger {
    capacity: std::num::NonZeroU32,
    entries: Mutex<BTreeMap<overdrive_core::public_ingress::SocketCookie, GatewayCleanupRecord>>,
    saturation_count: AtomicU64,
}

#[allow(dead_code, reason = "exact DISTILL RED cleanup record scaffold")]
pub(crate) struct GatewayCleanupRecord {
    intent: GatewayConnectIntent,
    connect_intent_failed: bool,
    receipt_failed: bool,
}

#[allow(dead_code, reason = "exact DISTILL RED cleanup reservation scaffold")]
pub(crate) struct GatewayCleanupReservation {
    ledger: Arc<GatewayCleanupLedger>,
    socket_cookie: overdrive_core::public_ingress::SocketCookie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GatewayCleanupLedgerFull {
    pub capacity: std::num::NonZeroU32,
}
#[allow(dead_code, reason = "exact DISTILL RED cleanup-ledger scaffold")]
impl GatewayCleanupLedger {
    pub(crate) fn new(_capacity: std::num::NonZeroU32) -> Arc<Self> {
        todo!("SCAFFOLD: GatewayCleanupLedger::new")
    }
    pub(crate) fn reserve(
        self: &Arc<Self>,
        _intent: GatewayConnectIntent,
    ) -> Result<GatewayCleanupReservation, GatewayCleanupLedgerFull> {
        todo!("SCAFFOLD: GatewayCleanupLedger::reserve")
    }
    pub(crate) fn retry_all(
        &self,
        _dataplane: &dyn GatewayConnectDataplane,
    ) -> Vec<GatewayCleanupFailure> {
        todo!("SCAFFOLD: GatewayCleanupLedger::retry_all")
    }
    pub(crate) fn len(&self) -> u32 {
        let _ = (&self.capacity, &self.entries, self.saturation_count.load(Ordering::Relaxed));
        todo!("SCAFFOLD: GatewayCleanupLedger::len")
    }
}

#[allow(dead_code, reason = "exact DISTILL RED cleanup reservation operations")]
impl GatewayCleanupReservation {
    pub(crate) fn record_intent_failure(&self) {
        let _ = (&self.ledger, self.socket_cookie);
        todo!("SCAFFOLD: GatewayCleanupReservation::record_intent_failure")
    }
    pub(crate) fn record_receipt_failure(&self) {
        todo!("SCAFFOLD: GatewayCleanupReservation::record_receipt_failure")
    }
    pub(crate) fn complete(self) {
        todo!("SCAFFOLD: GatewayCleanupReservation::complete")
    }
}

impl UnboundGatewayBuilder {
    pub async fn new(
        _config: crate::GatewayConfig,
        _intent: Arc<dyn overdrive_core::traits::IntentStore>,
        _observations: Arc<dyn overdrive_core::traits::ObservationStore>,
        _service_vip_view: Arc<dyn overdrive_core::traits::ServiceVipView>,
        _key_codec: Arc<overdrive_host::ca::PublicCertifiedKeyAeadCodec>,
        _cookie_reader: Arc<overdrive_host::socket_cookie::SocketCookieReader>,
        _clock: Arc<dyn overdrive_core::traits::Clock>,
    ) -> Result<Self, GatewayBootError> {
        todo!("SCAFFOLD: UnboundGatewayBuilder::new")
    }

    pub fn bind_demand_wake(
        self,
        _wake: Arc<dyn GatewayFrontendDemandWake>,
    ) -> (GatewayBuilder, GatewayDemandComposition, crate::ports::GatewayUpstreamSeal) {
        todo!("SCAFFOLD: UnboundGatewayBuilder::bind_demand_wake")
    }
}

impl GatewayBuilder {
    pub async fn start(self, _dependencies: GatewayRuntimeDependencies) -> StartedGateway {
        todo!("SCAFFOLD: GatewayBuilder::start")
    }
}

impl StartedGateway {
    pub fn into_parts(self) -> (GatewayHandle, GatewayControl) {
        todo!("SCAFFOLD: StartedGateway::into_parts")
    }
}

impl GatewayHandle {
    pub async fn shutdown(self, _grace: Duration) -> GatewayShutdownReport {
        todo!("SCAFFOLD: GatewayHandle::shutdown")
    }
}

#[derive(Debug, Default)]
pub struct GatewayShutdownReport {
    pub forced_connections: u32,
    pub task_failures: Vec<GatewayTaskFailure>,
    pub cleanup_failures: Vec<GatewayCleanupFailure>,
    pub application_errors: Vec<GatewayApplicationCommandError>,
    pub identity_error: Option<crate::ports::GatewayIdentityLifecycleError>,
    pub identity_empty: bool,
    pub residual_connect: Option<GatewayResidualConnectState>,
}
impl GatewayShutdownReport {
    pub fn is_clean(&self) -> bool {
        todo!("SCAFFOLD: GatewayShutdownReport::is_clean")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayTaskName {
    Listener,
    Connections,
    Application,
    Demand,
    ManualCertifiedKeySource,
    PublicCertifiedKeyCustody,
    PublicRouteSet,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayTaskFailureCause {
    OwnerFailed,
    ChannelClosed,
    Timeout,
    Cancelled,
    Panicked,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayTaskFailure {
    pub task: GatewayTaskName,
    pub cause: GatewayTaskFailureCause,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayCleanupFailure {
    ConnectIntent { count: u32 },
    Receipt { count: u32 },
    LedgerSaturated { attempts: u64, capacity: u32 },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayApplicationCommandPhase {
    Close,
    Retire,
}
#[derive(Debug, Error)]
pub enum GatewayApplicationCommandError {
    #[error("Gateway Application owner unavailable")]
    OwnerUnavailable,
    #[error("Gateway Application command timed out during {phase:?}")]
    Timeout { phase: GatewayApplicationCommandPhase },
    #[error("Gateway frontend demand failed: {0}")]
    Demand(GatewayFrontendDemandError),
}
#[derive(Debug)]
pub struct GatewayResidualConnectState {
    pub live_intents: Option<u32>,
    pub live_receipts: Option<u32>,
    pub sweep_failures: Vec<GatewayConnectSweepFailure>,
    pub ledger_entries: u32,
}
#[derive(Debug)]
pub enum GatewayConnectSweepFailure {
    CleanupAll(crate::ports::GatewayConnectDataplaneError),
    IntentCount(crate::ports::GatewayConnectDataplaneError),
    ReceiptCount(crate::ports::GatewayConnectDataplaneError),
}

#[derive(Debug)]
#[expect(dead_code, reason = "closed admission variants activate incrementally")]
pub(crate) enum GatewayAdmissionError {
    Unavailable,
    CertifiedKeyUnusable,
    ConnectionLimit,
    ConnectLimit,
    RequestLimit,
}

#[derive(Debug, Error)]
pub enum GatewayBootError {
    #[error("gateway configuration failed: {0}")]
    Configuration(crate::GatewayConfigError),
    #[error("Route store failed: {0}")]
    RouteStore(PublicRouteSetError),
    #[error("public certified-key custody failed: {0}")]
    CertifiedKey(crate::certified_key::PublicCertifiedKeyError),
    #[error("manual public certified-key source failed: {0}")]
    ManualCertifiedKey(crate::certified_key::PublicCertifiedKeyError),
}

#[cfg(test)]
mod completeness {
    use super::*;
    use overdrive_core::dataplane::{Proto, ServiceFrontend};
    use overdrive_core::id::ServiceVip;
    use overdrive_core::public_ingress::{ServiceKey, SocketCookie};

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[ignore = "pending DELIVER bounded cleanup-ledger admission"]
    fn cleanup_ledger_accepts_zero_one_and_limit_then_refuses_limit_plus_one() {
        let capacity = std::num::NonZeroU32::new(128).expect("fixed capacity");
        let ledger = GatewayCleanupLedger::new(capacity);
        assert_eq!(ledger.len(), 0);
        let frontend = ServiceFrontend::new(
            ServiceVip::new("10.96.0.8".parse().expect("IPv4")).expect("VIP"),
            std::num::NonZeroU16::new(8080).expect("port"),
            Proto::Tcp,
        )
        .expect("frontend");
        let mut reservations = Vec::new();
        for cookie in 1..=capacity.get() {
            reservations.push(
                ledger
                    .reserve(GatewayConnectIntent {
                        socket_cookie: SocketCookie::new(u64::from(cookie))
                            .expect("nonzero cookie"),
                        service_key: ServiceKey::from_frontend(frontend),
                    })
                    .expect("within capacity"),
            );
        }
        assert_eq!(ledger.len(), capacity.get());
        let overflow = match ledger.reserve(GatewayConnectIntent {
            socket_cookie: SocketCookie::new(u64::from(capacity.get()) + 1).expect("cookie"),
            service_key: ServiceKey::from_frontend(frontend),
        }) {
            Ok(_) => panic!("limit plus one is refused before BPF registration"),
            Err(error) => error,
        };
        assert_eq!(overflow.capacity, capacity);
        for reservation in reservations {
            reservation.complete();
        }
        assert_eq!(ledger.len(), 0);
    }
}
