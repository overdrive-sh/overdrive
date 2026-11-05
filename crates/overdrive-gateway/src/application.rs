//! Gateway Application owner, composition surface and admission state.
//!
//! SCAFFOLD: true. The DESIGN-pinned public surface (builder, handle,
//! route-command and status-read ports, shutdown report) and the crate-private admission / ledger shapes
//! the acceptance bodies reach compile today; every owner behaviour body is
//! DELIVER's. Source-local acceptance bodies implement S-PIG-37 and S-PIG-73;
//! the QC-2 conformance driver is the child module below.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use arc_swap::ArcSwapOption;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::public_ingress::{
    GatewayApplicationGenerationId, GatewayApplicationStatusRowV1, PublicCertifiedKeyStatusRowV1,
    Route,
};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::ObservationStoreError;
use overdrive_core::traits::{IntentStore, ObservationStore, ServiceVipView};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_host::ca::PublicCertifiedKeyAeadCodec;
use thiserror::Error;
use tokio::sync::Notify;

use crate::certified_key::{PublicCertifiedKeyError, PublicCertifiedKeySnapshot};
use crate::connector::GatewayUpstreamConnectError;
use crate::ports::{
    GatewayClientMtls, GatewayConnectCleanup, GatewayConnectDataplane,
    GatewayConnectDataplaneError, GatewayConnectIntent, GatewayIdentityLifecycleControl,
    GatewayIdentityLifecycleError, GatewayRelay, GatewayUpstreamSeal, ServiceFrontendResolveError,
};
use crate::route_set::{PublicRouteSetError, PublicRouteSetHandle};
use crate::runtime::GatewayLimits;
use crate::{GatewayConfig, GatewayConfigError};

#[cfg(all(test, feature = "integration-tests"))]
mod held_probe_doubles;
#[cfg(test)]
mod ledger_acceptance;
#[cfg(all(test, feature = "integration-tests"))]
mod probe_completion_acceptance;
#[cfg(all(test, feature = "integration-tests"))]
mod quint_gateway_application;

// ---------------------------------------------------------------------------
// Applied generation
// ---------------------------------------------------------------------------

/// The authority that one coherent Gateway Application generation was
/// promoted. Constructible only inside the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct GatewayAppliedGeneration {
    application_generation: GatewayApplicationGenerationId,
}

impl GatewayAppliedGeneration {
    pub(crate) const fn from_promotion(
        application_generation: GatewayApplicationGenerationId,
    ) -> Self {
        Self { application_generation }
    }
    pub const fn application_generation(&self) -> GatewayApplicationGenerationId {
        self.application_generation
    }
}

// ---------------------------------------------------------------------------
// Public builder, handle, route-command and status-read ports
// ---------------------------------------------------------------------------

pub struct UnboundGatewayBuilder {
    _private: (),
}

impl UnboundGatewayBuilder {
    pub async fn new(
        config: GatewayConfig,
        intent: Arc<dyn IntentStore>,
        observations: Arc<dyn ObservationStore>,
        service_vip_view: Arc<dyn ServiceVipView>,
        key_codec: Arc<PublicCertifiedKeyAeadCodec>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, GatewayBootError> {
        todo!("SCAFFOLD: UnboundGatewayBuilder::new")
    }

    pub fn into_builder(self) -> (GatewayBuilder, GatewayUpstreamSeal) {
        todo!("SCAFFOLD: UnboundGatewayBuilder::into_builder")
    }
}

pub struct GatewayRuntimeDependencies {
    _private: (),
}

impl GatewayRuntimeDependencies {
    pub fn new(
        limits: GatewayLimits,
        dataplane: Arc<dyn GatewayConnectDataplane>,
        client_mtls: Arc<dyn GatewayClientMtls>,
        identity_lifecycle: Arc<dyn GatewayIdentityLifecycleControl>,
        relay: Arc<dyn GatewayRelay>,
    ) -> Self {
        todo!("SCAFFOLD: GatewayRuntimeDependencies::new")
    }
}

pub struct GatewayBuilder {
    _private: (),
}

impl GatewayBuilder {
    /// Exactly: build the one budget from `dependencies`' limits,
    /// `GatewayAdmission::new(budget)`, `start_with`.
    pub async fn start(self, dependencies: GatewayRuntimeDependencies) -> StartedGateway {
        todo!("SCAFFOLD: GatewayBuilder::start")
    }

    /// Crate-private (DG-10): `start` with an injected admission. The
    /// admission's budget was built from the same limits as `dependencies`.
    pub(crate) async fn start_with(
        self,
        dependencies: GatewayRuntimeDependencies,
        admission: Arc<GatewayAdmission>,
    ) -> StartedGateway {
        todo!("SCAFFOLD: GatewayBuilder::start_with")
    }
}

pub struct StartedGateway {
    _private: (),
}

impl StartedGateway {
    pub fn into_parts(self) -> (GatewayHandle, GatewayRouteCommands, GatewayStatusReads) {
        todo!("SCAFFOLD: StartedGateway::into_parts")
    }
}

/// Route command port (R-25): the write handle of the Public Route Set; it
/// exposes no status read.
#[derive(Clone)]
pub struct GatewayRouteCommands {
    _private: (),
}

impl GatewayRouteCommands {
    pub fn routes(&self) -> &PublicRouteSetHandle {
        todo!("SCAFFOLD: GatewayRouteCommands::routes")
    }
}

/// Read-only status port (R-25): redacted ObservationStore point reads; no
/// Route write capability is reachable from it.
#[derive(Clone)]
pub struct GatewayStatusReads {
    _private: (),
}

impl GatewayStatusReads {
    pub async fn application_status(
        &self,
    ) -> Result<Option<GatewayApplicationStatusRowV1>, GatewayStatusReadError> {
        todo!("SCAFFOLD: GatewayStatusReads::application_status")
    }
    pub async fn certified_key_status(
        &self,
    ) -> Result<Option<PublicCertifiedKeyStatusRowV1>, GatewayStatusReadError> {
        todo!("SCAFFOLD: GatewayStatusReads::certified_key_status")
    }
}

#[derive(Debug, Error)]
pub enum GatewayStatusReadError {
    #[error("Gateway Application status read failed: {0}")]
    Application(ObservationStoreError),
    #[error("public certified-key status read failed: {0}")]
    CertifiedKey(ObservationStoreError),
}

pub struct GatewayHandle {
    _private: (),
}

impl GatewayHandle {
    pub async fn shutdown(self, grace: Duration) -> GatewayShutdownReport {
        todo!("SCAFFOLD: GatewayHandle::shutdown")
    }
}

#[derive(Debug)]
pub struct GatewayShutdownReport {
    /// Time the stage consumed on the injected clock.
    pub elapsed: Duration,
    pub forced_connections: u32,
    pub task_failures: Vec<GatewayTaskFailure>,
    pub cleanup_failures: Vec<GatewayCleanupFailure>,
    pub application_errors: Vec<GatewayApplicationCommandError>,
    pub identity_empty: bool,
    pub residual_connect: Option<GatewayResidualConnectState>,
}

impl GatewayShutdownReport {
    pub fn is_clean(&self) -> bool {
        self.forced_connections == 0
            && self.task_failures.is_empty()
            && self.cleanup_failures.is_empty()
            && self.application_errors.is_empty()
            && self.identity_empty
            && self.residual_connect.is_none()
    }
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
    CleanupAll(GatewayConnectDataplaneError),
    IntentCount(GatewayConnectDataplaneError),
    // No receipt-count variant: the residual receipt count is `live_receipts()?.len()` (R-33); a
    // failed `live_receipts()` read leaves `GatewayResidualConnectState::live_receipts` as `None`,
    // so the report is not clean.
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayTaskName {
    Listener,
    Connections,
    Application,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum GatewayApplicationCommandError {
    #[error("Gateway Application owner unavailable")]
    OwnerUnavailable,
    #[error("Gateway Application command timed out during {phase:?}")]
    Timeout { phase: GatewayApplicationCommandPhase },
}

#[derive(Debug, Error)]
pub enum GatewayBootError {
    #[error("gateway configuration failed: {0}")]
    Configuration(GatewayConfigError),
    #[error("Route store failed: {0}")]
    RouteStore(PublicRouteSetError),
    #[error("public certified-key custody failed: {0}")]
    CertifiedKey(PublicCertifiedKeyError),
    #[error("manual public certified-key source failed: {0}")]
    ManualCertifiedKey(PublicCertifiedKeyError),
}

// ---------------------------------------------------------------------------
// Crate-private candidate, snapshot and retention
// ---------------------------------------------------------------------------

pub(crate) struct GatewayApplicationCandidate {
    generation: GatewayApplicationGenerationId,
    route: Arc<Route>,
    frontend: ServiceFrontend,
    certified_key: Arc<PublicCertifiedKeySnapshot>,
}

impl GatewayApplicationCandidate {
    pub(crate) fn new(
        route: Arc<Route>,
        frontend: ServiceFrontend,
        certified_key: Arc<PublicCertifiedKeySnapshot>,
    ) -> Result<Self, GatewayApplicationError> {
        todo!("SCAFFOLD: GatewayApplicationCandidate::new")
    }
}

pub(crate) struct GatewayApplicationSnapshot {
    applied: GatewayAppliedGeneration,
    route: Arc<Route>,
    frontend: ServiceFrontend,
    certified_key: Arc<PublicCertifiedKeySnapshot>,
    retention: Arc<GatewayGenerationRetention>,
    budget: Arc<GatewayAdmissionBudget>,
}

impl GatewayApplicationSnapshot {
    pub(crate) const fn applied(&self) -> GatewayAppliedGeneration {
        self.applied
    }
    pub(crate) fn route(&self) -> &Route {
        &self.route
    }
    pub(crate) const fn frontend(&self) -> ServiceFrontend {
        self.frontend
    }
    pub(crate) fn certified_key(&self) -> &PublicCertifiedKeySnapshot {
        &self.certified_key
    }
}

pub(crate) struct GatewayGenerationRetention {
    generation: GatewayApplicationGenerationId,
    closed: AtomicBool,
    connections: AtomicU32,
    requests: AtomicU32,
    owner_wake: Arc<Notify>,
}

impl GatewayGenerationRetention {
    pub(crate) fn new(
        generation: GatewayApplicationGenerationId,
        owner_wake: Arc<Notify>,
    ) -> Arc<Self> {
        todo!("SCAFFOLD: GatewayGenerationRetention::new")
    }
    pub(crate) fn try_connection(
        self: &Arc<Self>,
    ) -> Result<GatewayGenerationConnectionPermit, GatewayAdmissionError> {
        todo!("SCAFFOLD: GatewayGenerationRetention::try_connection")
    }
    pub(crate) fn try_request(
        self: &Arc<Self>,
    ) -> Result<GatewayGenerationRequestPermit, GatewayAdmissionError> {
        todo!("SCAFFOLD: GatewayGenerationRetention::try_request")
    }
    pub(crate) fn close(&self) {
        todo!("SCAFFOLD: GatewayGenerationRetention::close")
    }
    pub(crate) async fn wait_zero(
        &self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Result<(), GatewayDrainTimeout> {
        todo!("SCAFFOLD: GatewayGenerationRetention::wait_zero")
    }
    pub(crate) fn counts(&self) -> (u32, u32) {
        todo!("SCAFFOLD: GatewayGenerationRetention::counts")
    }
}

pub(crate) struct GatewayGenerationConnectionPermit {
    retention: Arc<GatewayGenerationRetention>,
}

pub(crate) struct GatewayGenerationRequestPermit {
    retention: Arc<GatewayGenerationRetention>,
}

// ---------------------------------------------------------------------------
// Admission budget, admission and leases
// ---------------------------------------------------------------------------

pub(crate) struct GatewayAdmissionBudget {
    limits: Arc<GatewayLimits>,
    public_connections: AtomicU32,
    inflight_connects: AtomicU32,
    inflight_requests: AtomicU32,
}

impl GatewayAdmissionBudget {
    pub(crate) fn new(limits: Arc<GatewayLimits>) -> Arc<Self> {
        Arc::new(Self {
            limits,
            public_connections: AtomicU32::new(0),
            inflight_connects: AtomicU32::new(0),
            inflight_requests: AtomicU32::new(0),
        })
    }
    pub(crate) fn try_public_connection(
        self: &Arc<Self>,
    ) -> Result<GatewayPublicConnectionPermit, GatewayAdmissionError> {
        todo!("SCAFFOLD: GatewayAdmissionBudget::try_public_connection")
    }
    pub(crate) fn try_connect(
        self: &Arc<Self>,
    ) -> Result<GatewayConnectPermit, GatewayAdmissionError> {
        let ceiling = self.limits.max_inflight_connects().get();
        let taken =
            self.inflight_connects.fetch_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                (held < ceiling).then_some(held + 1)
            });
        match taken {
            Ok(_) => Ok(GatewayConnectPermit { budget: Arc::clone(self) }),
            Err(_) => todo!(
                "SCAFFOLD: GatewayAdmissionBudget::try_connect refusal arm (admission refusal)"
            ),
        }
    }
    pub(crate) fn try_request(
        self: &Arc<Self>,
    ) -> Result<GatewayGlobalRequestPermit, GatewayAdmissionError> {
        todo!("SCAFFOLD: GatewayAdmissionBudget::try_request")
    }
}

pub(crate) struct GatewayPublicConnectionPermit {
    budget: Arc<GatewayAdmissionBudget>,
}

pub(crate) struct GatewayConnectPermit {
    budget: Arc<GatewayAdmissionBudget>,
}

impl Drop for GatewayConnectPermit {
    fn drop(&mut self) {
        self.budget.inflight_connects.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) struct GatewayGlobalRequestPermit {
    budget: Arc<GatewayAdmissionBudget>,
}

pub(crate) struct GatewayAdmission {
    current: Arc<ArcSwapOption<GatewayApplicationSnapshot>>,
    budget: Arc<GatewayAdmissionBudget>,
}

impl GatewayAdmission {
    /// `current` empty. The one constructor (DG-10).
    pub(crate) fn new(budget: Arc<GatewayAdmissionBudget>) -> Arc<Self> {
        todo!("SCAFFOLD: GatewayAdmission::new")
    }
    /// The budget shared with the connector and every generation snapshot.
    pub(crate) fn budget(&self) -> &Arc<GatewayAdmissionBudget> {
        &self.budget
    }
    pub(crate) fn acquire_connection(
        &self,
        now: UnixInstant,
    ) -> Result<GatewayConnectionLease, GatewayAdmissionError> {
        todo!("SCAFFOLD: GatewayAdmission::acquire_connection")
    }
}

/// One admitted public connection; retains one whole Current snapshot.
/// Neither lease implements `Clone` or `Default`.
pub(crate) struct GatewayConnectionLease {
    snapshot: Arc<GatewayApplicationSnapshot>,
    generation: GatewayGenerationConnectionPermit,
    public_connection: GatewayPublicConnectionPermit,
    requests_started: AtomicU32,
    request_active: Arc<AtomicBool>,
}

impl GatewayConnectionLease {
    pub(crate) fn snapshot(&self) -> &GatewayApplicationSnapshot {
        &self.snapshot
    }
    pub(crate) fn try_request(&self) -> Result<GatewayRequestLease, GatewayAdmissionError> {
        todo!("SCAFFOLD: GatewayConnectionLease::try_request")
    }
}

pub(crate) struct GatewayRequestLease {
    snapshot: Arc<GatewayApplicationSnapshot>,
    generation: GatewayGenerationRequestPermit,
    global: GatewayGlobalRequestPermit,
    request_active: Arc<AtomicBool>,
    close_after_response: bool,
}

impl GatewayRequestLease {
    pub(crate) fn snapshot(&self) -> &GatewayApplicationSnapshot {
        &self.snapshot
    }
    pub(crate) const fn close_after_response(&self) -> bool {
        self.close_after_response
    }
}

#[derive(Debug)]
pub(crate) enum GatewayAdmissionError {
    Unavailable,
    CertifiedKeyUnusable,
    ConnectionLimit,
    ConnectLimit,
    RequestLimit,
}

pub(crate) struct GatewayDrainTimeout {
    pub generation: GatewayApplicationGenerationId,
    pub connections: u32,
    pub requests: u32,
}

#[derive(Debug)]
pub(crate) enum GatewayApplicationError {
    CertifiedKeyReferenceMismatch,
    CertifiedKeyHostnameMismatch,
    Frontend(ServiceFrontendResolveError),
    Identity(GatewayIdentityLifecycleError),
    UpstreamProbe(GatewayUpstreamConnectError),
    Bind(std::io::ErrorKind),
    WatchGap,
    Superseded,
}

// ---------------------------------------------------------------------------
// Bounded cleanup ledger (R-5, DG-11: reserve before `open_upstream`, bind after)
// ---------------------------------------------------------------------------

/// One slot of the ledger: reserved and not yet bound, or the record of a
/// registered intent.
enum LedgerSlot {
    Reserved,
    Bound(GatewayCleanupRecord),
}

pub(crate) struct GatewayCleanupLedger {
    capacity: NonZeroU32,
    saturation_count: AtomicU64,
    slots: Mutex<LedgerSlots>,
}

#[derive(Default)]
struct LedgerSlots {
    next_slot: u64,
    by_slot: BTreeMap<u64, LedgerSlot>,
}

impl LedgerSlots {
    /// Takes one slot while fewer than `capacity` are held.
    fn reserve_below(&mut self, capacity: NonZeroU32) -> Option<u64> {
        let held = u32::try_from(self.by_slot.len()).unwrap_or(u32::MAX);
        if held >= capacity.get() {
            return None;
        }
        let slot = self.next_slot;
        self.next_slot = self.next_slot.saturating_add(1);
        self.by_slot.insert(slot, LedgerSlot::Reserved);
        Some(slot)
    }
}

pub(crate) struct GatewayCleanupRecord {
    intent: GatewayConnectIntent,
    connect_intent_failed: bool,
    receipt_failed: bool,
}

pub(crate) struct GatewayCleanupReservation {
    ledger: Arc<GatewayCleanupLedger>,
    slot: u64,
    bound: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GatewayCleanupLedgerFull {
    pub capacity: NonZeroU32,
}

impl GatewayCleanupLedger {
    pub(crate) fn new(capacity: NonZeroU32) -> Arc<Self> {
        Arc::new(Self {
            capacity,
            saturation_count: AtomicU64::new(0),
            slots: Mutex::new(LedgerSlots::default()),
        })
    }

    fn slots(&self) -> std::sync::MutexGuard<'_, LedgerSlots> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes one slot before `open_upstream`; the slot is bound to the intent
    /// afterwards. Dropping a reservation that was never bound releases it.
    /// A full ledger counts one saturation and refuses.
    pub(crate) fn reserve(
        self: &Arc<Self>,
    ) -> Result<GatewayCleanupReservation, GatewayCleanupLedgerFull> {
        let Some(slot) = self.slots().reserve_below(self.capacity) else {
            self.saturation_count
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                    Some(count.saturating_add(1))
                })
                .unwrap_or_else(|_| unreachable!("the update closure always returns Some"));
            return Err(GatewayCleanupLedgerFull { capacity: self.capacity });
        };
        Ok(GatewayCleanupReservation { ledger: Arc::clone(self), slot, bound: false })
    }

    /// Cleans the exact intent of every retained record. The mutex guard is
    /// dropped before any cleanup call; only successfully cleaned records are
    /// removed.
    pub(crate) fn retry_all(
        &self,
        cleanup: &dyn GatewayConnectCleanup,
    ) -> Vec<GatewayCleanupFailure> {
        let snapshot: Vec<(u64, GatewayConnectIntent, bool, bool)> = self
            .slots()
            .by_slot
            .iter()
            .filter_map(|(slot, entry)| match entry {
                LedgerSlot::Bound(record) => Some((
                    *slot,
                    record.intent.clone(),
                    record.connect_intent_failed,
                    record.receipt_failed,
                )),
                LedgerSlot::Reserved => None,
            })
            .collect();
        let (mut intent_failures, mut receipt_failures) = (0_u32, 0_u32);
        let mut cleaned = Vec::with_capacity(snapshot.len());
        for (slot, intent, intent_failed, receipt_failed) in snapshot {
            if cleanup.cleanup(&intent).is_ok() {
                cleaned.push(slot);
            } else {
                intent_failures += u32::from(intent_failed || !receipt_failed);
                receipt_failures += u32::from(receipt_failed);
            }
        }
        let mut slots = self.slots();
        for slot in cleaned {
            slots.by_slot.remove(&slot);
        }
        drop(slots);
        let mut failures = Vec::with_capacity(3);
        if intent_failures > 0 {
            failures.push(GatewayCleanupFailure::ConnectIntent { count: intent_failures });
        }
        if receipt_failures > 0 {
            failures.push(GatewayCleanupFailure::Receipt { count: receipt_failures });
        }
        let attempts = self.saturation_count.load(Ordering::Acquire);
        if attempts > 0 {
            failures.push(GatewayCleanupFailure::LedgerSaturated {
                attempts,
                capacity: self.capacity.get(),
            });
        }
        failures
    }

    pub(crate) fn len(&self) -> u32 {
        u32::try_from(self.slots().by_slot.len()).unwrap_or(u32::MAX)
    }
}

impl GatewayCleanupReservation {
    /// Binds the reserved slot to the intent `open_upstream` registered.
    pub(crate) fn bind(&mut self, intent: GatewayConnectIntent) {
        self.ledger.slots().by_slot.insert(
            self.slot,
            LedgerSlot::Bound(GatewayCleanupRecord {
                intent,
                connect_intent_failed: false,
                receipt_failed: false,
            }),
        );
        self.bound = true;
    }
    pub(crate) fn record_intent_failure(&self) {
        todo!("SCAFFOLD: GatewayCleanupReservation::record_intent_failure")
    }
    pub(crate) fn record_receipt_failure(&self) {
        todo!("SCAFFOLD: GatewayCleanupReservation::record_receipt_failure")
    }
    /// Removes the record: both keys are known clean.
    pub(crate) fn complete(mut self) {
        self.ledger.slots().by_slot.remove(&self.slot);
        self.bound = true;
    }
}

impl Drop for GatewayCleanupReservation {
    fn drop(&mut self) {
        if !self.bound {
            self.ledger.slots().by_slot.remove(&self.slot);
        }
    }
}
