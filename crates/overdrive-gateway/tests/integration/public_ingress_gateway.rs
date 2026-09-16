//! S-PIG-21 through S-PIG-27 — Gateway Application integration scenarios.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use std::collections::BTreeMap;
use std::os::fd::OwnedFd;
use std::os::unix::fs::PermissionsExt as _;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, ServiceV1, VmInput, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::dataplane::Proto;
use overdrive_core::gateway_identity::{GatewayIdentityEpoch, GatewayIdentityFacts};
use overdrive_core::id::{BackendId, CertSerial, ContentHash, NodeId, ServiceVip, SpiffeId};
use overdrive_core::public_ingress::{
    GatewayApplicationStatusRowV1, GatewayApplicationUnavailableCause, GatewayConnectIntent,
    GatewaySelectionReceipt, PublicCertifiedKeyId, PublicRouteInput, Route, RouteApplyOutcome,
    RouteId, RouteWithdrawOutcome, ServiceListenerReferenceInput,
};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_gateway::application::{
    GatewayBootError, GatewayControl, GatewayHandle, GatewayRuntimeDependencies,
    UnboundGatewayBuilder,
};
use overdrive_gateway::certified_key::{PublicCertifiedKeyAeadError, PublicCertifiedKeyError};
use overdrive_gateway::ports::{
    GatewayClientMtls, GatewayClientMtlsError, GatewayConnectCleanupSweep, GatewayConnectDataplane,
    GatewayConnectDataplaneError, GatewayIdentityLifecycleControl, GatewayIdentityLifecycleError,
    GatewayIdentityWaitPhase, GatewayUpstream, GatewayUpstreamSeal,
};
use overdrive_gateway::route_set::PublicRouteSetError;
use overdrive_gateway::runtime::GatewayLimits;
use overdrive_gateway::{GatewayConfig, ManualCertifiedKeyConfig};
use overdrive_host::ca::PublicCertifiedKeyAeadCodec;
use overdrive_host::socket_cookie::SocketCookieReader;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::kek::SimKek;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::read_ports::SimServiceVipView;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;
use tokio::sync::watch;

struct NoopWake;
impl overdrive_core::public_ingress::GatewayFrontendDemandWake for NoopWake {
    fn wake(&self, _service_id: overdrive_core::id::ServiceId) {}
}

struct NoBackendDataplane;

#[async_trait]
impl GatewayConnectDataplane for NoBackendDataplane {
    async fn probe(
        &self,
        _deadline: Instant,
        _clock: &dyn Clock,
    ) -> Result<(), GatewayConnectDataplaneError> {
        Ok(())
    }

    fn register(&self, _intent: GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError> {
        Ok(())
    }

    fn take_receipt(
        &self,
        intent: &GatewayConnectIntent,
    ) -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError> {
        Ok(GatewaySelectionReceipt::NoBackend {
            socket_cookie: intent.socket_cookie,
            service_key: intent.service_key,
        })
    }

    fn cleanup(&self, _intent: &GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError> {
        Ok(())
    }

    fn cleanup_all_gateway_intents(
        &self,
    ) -> Result<GatewayConnectCleanupSweep, GatewayConnectDataplaneError> {
        Ok(GatewayConnectCleanupSweep { removed_intents: 0, removed_receipts: 0 })
    }

    fn selected_backend_identity(
        &self,
        _backend_id: BackendId,
    ) -> Result<SpiffeId, GatewayConnectDataplaneError> {
        SpiffeId::new("spiffe://overdrive.local/workload/api/alloc/api-0")
            .map_err(|_| GatewayConnectDataplaneError::Unavailable)
    }

    fn live_intent_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
        Ok(0)
    }
    fn live_receipt_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
        Ok(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GatewayDataplaneFaultOperation {
    Probe,
    Register,
    Receipt,
    SelectedIdentity,
    Cleanup,
    CleanupAll,
    IntentCount,
    ReceiptCount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GatewayDataplaneFaultCase {
    error: GatewayConnectDataplaneError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GatewayDataplaneErrorContract {
    operation: GatewayDataplaneFaultOperation,
    public_cause: GatewayApplicationUnavailableCause,
}

/// The one C6 SSOT table for the closed GatewayConnectDataplaneError set.
///
/// Each entry names the existing driven-port operation that can originate the
/// typed error. `gateway_connect_error_contract` below is an exhaustive
/// compile guard: adding a production variant fails this test module until its
/// owner operation and both evidence layers are updated.
fn gateway_connect_error_cases() -> Vec<GatewayDataplaneFaultCase> {
    use GatewayConnectDataplaneError as Error;

    vec![
        GatewayDataplaneFaultCase { error: Error::Unavailable },
        GatewayDataplaneFaultCase { error: Error::ProbeTimeout },
        GatewayDataplaneFaultCase { error: Error::IntentRegistryFull },
        GatewayDataplaneFaultCase { error: Error::IntentAlreadyRegistered },
        GatewayDataplaneFaultCase { error: Error::IntentMissing },
        GatewayDataplaneFaultCase { error: Error::ReceiptMissing },
        GatewayDataplaneFaultCase { error: Error::ReceiptMismatch },
        GatewayDataplaneFaultCase {
            error: Error::BackendIdentityMissing {
                backend_id: BackendId::new(7).expect("BackendId"),
            },
        },
        GatewayDataplaneFaultCase { error: Error::Kernel },
        GatewayDataplaneFaultCase { error: Error::Cleanup },
    ]
}

const fn gateway_connect_error_contract(
    error: &GatewayConnectDataplaneError,
) -> GatewayDataplaneErrorContract {
    use GatewayConnectDataplaneError as Error;

    match error {
        Error::Unavailable | Error::ProbeTimeout | Error::Kernel => GatewayDataplaneErrorContract {
            operation: GatewayDataplaneFaultOperation::Probe,
            public_cause: GatewayApplicationUnavailableCause::ConnectPathUnavailable,
        },
        Error::IntentRegistryFull | Error::IntentAlreadyRegistered => {
            GatewayDataplaneErrorContract {
                operation: GatewayDataplaneFaultOperation::Register,
                public_cause: GatewayApplicationUnavailableCause::ConnectPathUnavailable,
            }
        }
        Error::IntentMissing | Error::ReceiptMissing | Error::ReceiptMismatch => {
            GatewayDataplaneErrorContract {
                operation: GatewayDataplaneFaultOperation::Receipt,
                public_cause: GatewayApplicationUnavailableCause::ConnectPathUnavailable,
            }
        }
        Error::BackendIdentityMissing { .. } => GatewayDataplaneErrorContract {
            operation: GatewayDataplaneFaultOperation::SelectedIdentity,
            public_cause: GatewayApplicationUnavailableCause::ConnectPathUnavailable,
        },
        Error::Cleanup => GatewayDataplaneErrorContract {
            operation: GatewayDataplaneFaultOperation::CleanupAll,
            public_cause: GatewayApplicationUnavailableCause::ConnectPathUnavailable,
        },
    }
}

/// Fault adapter used only through Gateway Application's existing driven port.
/// It records which owner operation crossed the boundary and injects the typed
/// error from that operation; tests never call an adapter method directly.
struct FaultingDataplane {
    case: GatewayDataplaneFaultCase,
    calls: Mutex<Vec<GatewayDataplaneFaultOperation>>,
    intents: Mutex<Vec<GatewayConnectIntent>>,
    observed_errors: Mutex<Vec<GatewayConnectDataplaneError>>,
}

impl FaultingDataplane {
    fn new(case: GatewayDataplaneFaultCase) -> Arc<Self> {
        Arc::new(Self {
            case,
            calls: Mutex::new(Vec::new()),
            intents: Mutex::new(Vec::new()),
            observed_errors: Mutex::new(Vec::new()),
        })
    }

    fn fault_at(
        &self,
        operation: GatewayDataplaneFaultOperation,
    ) -> Option<GatewayConnectDataplaneError> {
        self.calls.lock().expect("dataplane calls lock").push(operation);
        if gateway_connect_error_contract(&self.case.error).operation != operation {
            return None;
        }
        let error = self.case.error.clone();
        self.observed_errors.lock().expect("dataplane error lock").push(error.clone());
        Some(error)
    }

    fn observed_errors(&self) -> Vec<GatewayConnectDataplaneError> {
        self.observed_errors.lock().expect("dataplane error lock").clone()
    }

    fn calls(&self) -> Vec<GatewayDataplaneFaultOperation> {
        self.calls.lock().expect("dataplane calls lock").clone()
    }

    fn intents(&self) -> Vec<GatewayConnectIntent> {
        self.intents.lock().expect("dataplane intents lock").clone()
    }
}

#[async_trait]
impl GatewayConnectDataplane for FaultingDataplane {
    async fn probe(
        &self,
        _deadline: Instant,
        _clock: &dyn Clock,
    ) -> Result<(), GatewayConnectDataplaneError> {
        self.fault_at(GatewayDataplaneFaultOperation::Probe).map_or(Ok(()), Err)
    }

    fn register(&self, intent: GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError> {
        self.intents.lock().expect("dataplane intents lock").push(intent);
        self.fault_at(GatewayDataplaneFaultOperation::Register).map_or(Ok(()), Err)
    }

    fn take_receipt(
        &self,
        intent: &GatewayConnectIntent,
    ) -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError> {
        if let Some(error) = self.fault_at(GatewayDataplaneFaultOperation::Receipt) {
            return Err(error);
        }
        if gateway_connect_error_contract(&self.case.error).operation
            == GatewayDataplaneFaultOperation::SelectedIdentity
        {
            return Ok(GatewaySelectionReceipt::Selected {
                socket_cookie: intent.socket_cookie,
                service_key: intent.service_key,
                backend_id: BackendId::new(7).expect("BackendId"),
            });
        }
        Ok(GatewaySelectionReceipt::NoBackend {
            socket_cookie: intent.socket_cookie,
            service_key: intent.service_key,
        })
    }

    fn cleanup(&self, _intent: &GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError> {
        self.fault_at(GatewayDataplaneFaultOperation::Cleanup).map_or(Ok(()), Err)
    }

    fn cleanup_all_gateway_intents(
        &self,
    ) -> Result<GatewayConnectCleanupSweep, GatewayConnectDataplaneError> {
        self.fault_at(GatewayDataplaneFaultOperation::CleanupAll)
            .map_or(Ok(GatewayConnectCleanupSweep { removed_intents: 0, removed_receipts: 0 }), Err)
    }

    fn selected_backend_identity(
        &self,
        backend_id: BackendId,
    ) -> Result<SpiffeId, GatewayConnectDataplaneError> {
        if let Some(error) = self.fault_at(GatewayDataplaneFaultOperation::SelectedIdentity) {
            return Err(error);
        }
        SpiffeId::new("spiffe://overdrive.local/workload/api/alloc/api-0")
            .map_err(|_| GatewayConnectDataplaneError::BackendIdentityMissing { backend_id })
    }

    fn live_intent_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
        self.fault_at(GatewayDataplaneFaultOperation::IntentCount).map_or(Ok(0), Err)
    }

    fn live_receipt_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
        self.fault_at(GatewayDataplaneFaultOperation::ReceiptCount).map_or(Ok(0), Err)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GatewayIdentityFaultOperation {
    EnsureCurrent,
    DisableAfterDrain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GatewayIdentityFaultCase {
    error: GatewayIdentityLifecycleError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GatewayIdentityErrorContract {
    operation: GatewayIdentityFaultOperation,
    public_cause: GatewayApplicationUnavailableCause,
}

/// The one C6 SSOT table for the closed GatewayIdentityLifecycleError set.
///
/// Timeout is represented for both declared wait phases because each phase is
/// a distinct originating owner operation; the exhaustive classifier below
/// prevents a new lifecycle variant from silently escaping the matrix.
fn gateway_identity_error_cases() -> Vec<GatewayIdentityFaultCase> {
    use GatewayIdentityLifecycleError as Error;

    vec![
        GatewayIdentityFaultCase { error: Error::ReconcilerUnavailable },
        GatewayIdentityFaultCase { error: Error::EpochExhausted },
        GatewayIdentityFaultCase { error: Error::IssueFailed },
        GatewayIdentityFaultCase { error: Error::DropFailed },
        GatewayIdentityFaultCase {
            error: Error::Timeout { phase: GatewayIdentityWaitPhase::EnsureCurrent },
        },
        GatewayIdentityFaultCase {
            error: Error::Timeout { phase: GatewayIdentityWaitPhase::DisableAfterDrain },
        },
        GatewayIdentityFaultCase { error: Error::Closed },
    ]
}

const fn gateway_identity_error_contract(
    error: &GatewayIdentityLifecycleError,
) -> GatewayIdentityErrorContract {
    use GatewayIdentityLifecycleError as Error;

    match error {
        Error::ReconcilerUnavailable
        | Error::EpochExhausted
        | Error::IssueFailed
        | Error::Closed => GatewayIdentityErrorContract {
            operation: GatewayIdentityFaultOperation::EnsureCurrent,
            public_cause: GatewayApplicationUnavailableCause::GatewayIdentityUnusable,
        },
        Error::DropFailed => GatewayIdentityErrorContract {
            operation: GatewayIdentityFaultOperation::DisableAfterDrain,
            public_cause: GatewayApplicationUnavailableCause::GatewayIdentityUnusable,
        },
        Error::Timeout { phase: GatewayIdentityWaitPhase::EnsureCurrent } => {
            GatewayIdentityErrorContract {
                operation: GatewayIdentityFaultOperation::EnsureCurrent,
                public_cause: GatewayApplicationUnavailableCause::GatewayIdentityUnusable,
            }
        }
        Error::Timeout { phase: GatewayIdentityWaitPhase::DisableAfterDrain } => {
            GatewayIdentityErrorContract {
                operation: GatewayIdentityFaultOperation::DisableAfterDrain,
                public_cause: GatewayApplicationUnavailableCause::GatewayIdentityUnusable,
            }
        }
    }
}

struct FaultingIdentity {
    facts: GatewayIdentityFacts,
    sender: watch::Sender<Option<GatewayIdentityFacts>>,
    case: GatewayIdentityFaultCase,
    observed_errors: Mutex<Vec<GatewayIdentityLifecycleError>>,
}

impl FaultingIdentity {
    fn new(case: GatewayIdentityFaultCase) -> Arc<Self> {
        let facts = gateway_identity_facts();
        let (sender, _) = watch::channel(Some(facts.clone()));
        Arc::new(Self { facts, sender, case, observed_errors: Mutex::new(Vec::new()) })
    }

    fn observed_errors(&self) -> Vec<GatewayIdentityLifecycleError> {
        self.observed_errors.lock().expect("identity error lock").clone()
    }
}

#[async_trait]
impl GatewayIdentityLifecycleControl for FaultingIdentity {
    async fn ensure_current(
        &self,
        _deadline: Instant,
    ) -> Result<GatewayIdentityFacts, GatewayIdentityLifecycleError> {
        if gateway_identity_error_contract(&self.case.error).operation
            == GatewayIdentityFaultOperation::EnsureCurrent
        {
            let error = self.case.error.clone();
            self.observed_errors.lock().expect("identity error lock").push(error.clone());
            return Err(error);
        }
        Ok(self.facts.clone())
    }

    fn current(&self) -> Option<GatewayIdentityFacts> {
        (gateway_identity_error_contract(&self.case.error).operation
            != GatewayIdentityFaultOperation::EnsureCurrent)
            .then(|| self.facts.clone())
    }

    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>> {
        self.sender.subscribe()
    }

    async fn disable_after_drain(
        &self,
        _deadline: Instant,
    ) -> Result<(), GatewayIdentityLifecycleError> {
        if gateway_identity_error_contract(&self.case.error).operation
            == GatewayIdentityFaultOperation::DisableAfterDrain
        {
            let error = self.case.error.clone();
            self.observed_errors.lock().expect("identity error lock").push(error.clone());
            return Err(error);
        }
        self.sender.send_replace(None);
        Ok(())
    }
}

fn gateway_identity_facts() -> GatewayIdentityFacts {
    GatewayIdentityFacts {
        epoch: GatewayIdentityEpoch::new(1).expect("first epoch"),
        spiffe_id: SpiffeId::new("spiffe://overdrive.local/gateway/gateway-node")
            .expect("gateway SPIFFE ID"),
        serial: CertSerial::new("01").expect("serial"),
        not_after: overdrive_core::UnixInstant::from_unix_duration(Duration::from_secs(
            4_000_000_000,
        )),
    }
}

fn assert_no_internal_debug_tokens(rendered: &str, debug_error: &str) {
    let status_tokens = rendered
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    for error_token in debug_error
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|token| !token.is_empty())
    {
        assert!(
            !status_tokens.iter().any(|status_token| status_token == &error_token),
            "status leaked the internal error token {error_token}: {rendered}",
        );
    }
}

fn assert_gateway_connect_status_redacted(
    status: &GatewayApplicationStatusRowV1,
    case: &GatewayDataplaneFaultCase,
    dataplane: &FaultingDataplane,
    key_path: &std::path::Path,
) {
    assert_eq!(
        status.unavailable,
        Some(gateway_connect_error_contract(&case.error).public_cause),
        "the public Application status must use the closed redacted connect-path cause",
    );
    let rendered = format!("{status:?}");
    let debug_error = format!("{:?}", case.error);
    assert_no_internal_debug_tokens(&rendered, &debug_error);
    assert!(!rendered.contains(&case.error.to_string()), "status leaked the internal error text");
    if let GatewayConnectDataplaneError::BackendIdentityMissing { backend_id } = &case.error {
        assert!(!rendered.contains(&backend_id.to_string()), "status leaked BackendId");
    }
    for forbidden in [
        "gateway connect dataplane",
        "gateway intent",
        "gateway receipt",
        "gateway kernel operation",
        "api-origin-key.pem",
        "PRIVATE KEY",
        "spiffe://overdrive.local/workload/api/alloc/api-0",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "status leaked forbidden connect detail {forbidden}"
        );
    }
    assert!(
        !rendered.contains(&key_path.display().to_string()),
        "status leaked the configured private-key path",
    );
    for intent in dataplane.intents() {
        let socket_cookie = intent.socket_cookie.get().get().to_string();
        assert!(!rendered.contains(&socket_cookie), "status leaked socket cookie {socket_cookie}");
    }
}

fn assert_gateway_identity_status_redacted(
    status: &GatewayApplicationStatusRowV1,
    case: &GatewayIdentityFaultCase,
    key_path: &std::path::Path,
) {
    assert_eq!(
        status.unavailable,
        Some(gateway_identity_error_contract(&case.error).public_cause),
        "the public Application status must use the closed redacted identity cause",
    );
    let rendered = format!("{status:?}");
    let debug_error = format!("{:?}", case.error);
    assert_no_internal_debug_tokens(&rendered, &debug_error);
    assert!(
        !rendered.contains(&case.error.to_string()),
        "status leaked the internal identity text"
    );
    for forbidden in [
        "gateway identity reconciler unavailable",
        "gateway SVID issuance failed",
        "gateway SVID drop failed",
        "EnsureCurrent",
        "DisableAfterDrain",
        "PRIVATE KEY",
        "api-origin-key.pem",
        "spiffe://overdrive.local/workload/api/alloc/api-0",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "status leaked forbidden identity detail {forbidden}"
        );
    }
    assert!(
        !rendered.contains(&key_path.display().to_string()),
        "status leaked the configured private-key path",
    );
}

struct CurrentIdentity {
    facts: GatewayIdentityFacts,
    sender: watch::Sender<Option<GatewayIdentityFacts>>,
    disable_error: Mutex<Option<GatewayIdentityLifecycleError>>,
}

fn healthy_identity() -> Arc<CurrentIdentity> {
    let facts = gateway_identity_facts();
    let (sender, _) = watch::channel(Some(facts.clone()));
    Arc::new(CurrentIdentity { facts, sender, disable_error: Mutex::new(None) })
}

#[async_trait]
impl GatewayIdentityLifecycleControl for CurrentIdentity {
    async fn ensure_current(
        &self,
        _deadline: Instant,
    ) -> Result<GatewayIdentityFacts, GatewayIdentityLifecycleError> {
        Ok(self.facts.clone())
    }

    fn current(&self) -> Option<GatewayIdentityFacts> {
        Some(self.facts.clone())
    }

    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>> {
        self.sender.subscribe()
    }

    async fn disable_after_drain(
        &self,
        _deadline: Instant,
    ) -> Result<(), GatewayIdentityLifecycleError> {
        let error = self.disable_error.lock().expect("identity fault lock").take();
        if let Some(error) = error {
            return Err(error);
        }
        self.sender.send_replace(None);
        Ok(())
    }
}

struct MatchingMtls {
    seal: GatewayUpstreamSeal,
}

#[async_trait]
impl GatewayClientMtls for MatchingMtls {
    async fn authenticate(
        &self,
        connected: OwnedFd,
        _expected_peer: SpiffeId,
        _deadline: Instant,
    ) -> Result<GatewayUpstream, GatewayClientMtlsError> {
        drop(connected);
        let (stream, _peer) = tokio::io::duplex(1024);
        Ok(self.seal.seal_authenticated(Pin::from(Box::new(stream))))
    }
}

struct World {
    root: TempDir,
    chain_path: std::path::PathBuf,
    key_path: std::path::PathBuf,
    clock: Arc<SimClock>,
    intent_path: std::path::PathBuf,
    intent: Arc<LocalIntentStore>,
    control: GatewayControl,
    handle: GatewayHandle,
}

impl World {
    async fn start() -> Result<Self, GatewayBootError> {
        Self::start_with_identity_disable_error(None).await
    }

    async fn start_with_identity_disable_error(
        disable_error: Option<GatewayIdentityLifecycleError>,
    ) -> Result<Self, GatewayBootError> {
        let root = tempfile::tempdir().expect("isolated gateway root");
        let (chain, key) = write_test_cert(&root);
        let config = GatewayConfig::new(
            "127.0.0.1".parse().expect("IPv4"),
            ManualCertifiedKeyConfig::new(
                PublicCertifiedKeyId::new("api-origin").expect("key ID"),
                chain.clone(),
                key.clone(),
            ),
        );
        let intent_path = root.path().join("intent.redb");
        let intent = Arc::new(LocalIntentStore::open(&intent_path).expect("real intent store"));
        let observations = Arc::new(SimObservationStore::single_peer(
            NodeId::new("gateway-node").expect("node ID"),
            54,
        ));
        let vip_view = Arc::new(SimServiceVipView::new(BTreeMap::<ContentHash, ServiceVip>::new()));
        let sim_clock = Arc::new(SimClock::new());
        let clock: Arc<dyn Clock> = sim_clock.clone();
        let builder = UnboundGatewayBuilder::new(
            config,
            intent.clone(),
            observations,
            vip_view,
            Arc::new(PublicCertifiedKeyAeadCodec::new(Arc::new(SimKek::for_boot()))),
            Arc::new(SocketCookieReader::new()),
            Arc::clone(&clock),
        )
        .await?;
        let (builder, _demand, seal) = builder.bind_demand_wake(Arc::new(NoopWake));
        let facts = GatewayIdentityFacts {
            epoch: GatewayIdentityEpoch::new(1).expect("first epoch"),
            spiffe_id: SpiffeId::new("spiffe://overdrive.local/gateway/gateway-node")
                .expect("gateway SPIFFE ID"),
            serial: CertSerial::new("01").expect("serial"),
            not_after: overdrive_core::UnixInstant::from_unix_duration(Duration::from_secs(
                4_000_000_000,
            )),
        };
        let (sender, _) = watch::channel(Some(facts.clone()));
        let dependencies = GatewayRuntimeDependencies::new(
            GatewayLimits::first_slice(),
            Arc::new(NoBackendDataplane),
            Arc::new(MatchingMtls { seal }),
            Arc::new(CurrentIdentity { facts, sender, disable_error: Mutex::new(disable_error) }),
        );
        let (handle, control) = builder.start(dependencies).await.into_parts();
        Ok(Self {
            root,
            chain_path: chain,
            key_path: key,
            clock: sim_clock,
            intent_path,
            intent,
            control,
            handle,
        })
    }
}

struct C6World {
    _root: TempDir,
    key_path: std::path::PathBuf,
    control: GatewayControl,
    handle: GatewayHandle,
}

async fn persist_c6_service(intent: &LocalIntentStore) -> Arc<SimServiceVipView> {
    let service = ServiceV1::from_submit(ServiceSpecInput {
        id: "api".to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 10, memory_bytes: 16 * 1024 * 1024 },
        driver: DriverInput::Vm(VmInput {
            command: "/bin/true".to_owned(),
            args: Vec::new(),
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
        listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
        startup_probes: Vec::new(),
        readiness_probes: Vec::new(),
        liveness_probes: Vec::new(),
    })
    .expect("valid api Service input");
    let workload_id = service.id.clone();
    let service_intent = WorkloadIntent::Service(service);
    let service_digest = service_intent.spec_digest().expect("api Service digest");
    let archived = service_intent.archive_for_store().expect("archive api Service intent");
    intent
        .put(IntentKey::for_workload(&workload_id).as_bytes(), archived.as_ref())
        .await
        .expect("persist api Service through the real IntentStore");
    intent
        .put(
            IntentKey::for_workload_kind(&workload_id).as_bytes(),
            &[WorkloadKind::Service.discriminator_byte()],
        )
        .await
        .expect("persist api Service kind discriminator");
    let service_vip = ServiceVip::new("127.0.0.1".parse().expect("IPv4")).expect("Service VIP");
    Arc::new(SimServiceVipView::new(BTreeMap::<ContentHash, ServiceVip>::from([(
        service_digest,
        service_vip,
    )])))
}

impl C6World {
    async fn start_with_dependencies(
        dataplane: Arc<dyn GatewayConnectDataplane>,
        identity: Arc<dyn GatewayIdentityLifecycleControl>,
    ) -> Result<Self, GatewayBootError> {
        let root = tempfile::tempdir().expect("isolated C6 gateway root");
        let (chain, key) = write_test_cert(&root);
        let config = GatewayConfig::new(
            "127.0.0.1".parse().expect("IPv4"),
            ManualCertifiedKeyConfig::new(
                PublicCertifiedKeyId::new("api-origin").expect("key ID"),
                chain,
                key.clone(),
            ),
        );
        let intent = Arc::new(
            LocalIntentStore::open(root.path().join("intent.redb")).expect("real intent store"),
        );
        let vip_view = persist_c6_service(intent.as_ref()).await;
        let observations = Arc::new(SimObservationStore::single_peer(
            NodeId::new("gateway-node").expect("node ID"),
            54,
        ));
        let sim_clock = Arc::new(SimClock::new());
        let clock: Arc<dyn Clock> = sim_clock.clone();
        let builder = UnboundGatewayBuilder::new(
            config,
            intent,
            observations,
            vip_view,
            Arc::new(PublicCertifiedKeyAeadCodec::new(Arc::new(SimKek::for_boot()))),
            Arc::new(SocketCookieReader::new()),
            clock,
        )
        .await?;
        let (builder, _demand, seal) = builder.bind_demand_wake(Arc::new(NoopWake));
        let dependencies = GatewayRuntimeDependencies::new(
            GatewayLimits::first_slice(),
            dataplane,
            Arc::new(MatchingMtls { seal }),
            identity,
        );
        let (handle, control) = builder.start(dependencies).await.into_parts();
        Ok(Self { _root: root, key_path: key, control, handle })
    }
}

fn write_test_cert(root: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("test key");
    let mut params = rcgen::CertificateParams::new(vec!["api.example.com".to_owned()])
        .expect("certificate params");
    params.is_ca = rcgen::IsCa::NoCa;
    params.not_before = rcgen::date_time_ymd(2020, 1, 1);
    params.not_after = rcgen::date_time_ymd(2100, 1, 1);
    let cert = params.self_signed(&key).expect("test certificate");
    let chain_path = root.path().join("api-origin-chain.pem");
    let key_path = root.path().join("api-origin-key.pem");
    std::fs::write(&chain_path, cert.pem()).expect("write certificate");
    std::fs::write(&key_path, key.serialize_pem()).expect("write private key");
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
        .expect("private-key mode");
    assert_eq!(
        std::fs::metadata(&key_path).expect("key metadata").permissions().mode() & 0o777,
        0o600,
        "GatewayConfig fixture requires exact private-key mode",
    );
    (chain_path, key_path)
}

fn route(id: &str, path: &str) -> Route {
    Route::from_submit(PublicRouteInput {
        id: id.to_owned(),
        hostname: "api.example.com".to_owned(),
        path: path.to_owned(),
        path_match: "segment_prefix".to_owned(),
        certified_key: "api-origin".to_owned(),
        target: ServiceListenerReferenceInput {
            service: "api".to_owned(),
            port: 8080,
            protocol: Proto::Tcp.as_str().to_owned(),
        },
    })
    .expect("valid Route")
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER GatewayConnectDataplane owner error matrix"]
async fn every_gateway_connect_dataplane_error_crosses_its_real_owner_boundary() {
    for case in gateway_connect_error_cases() {
        let contract = gateway_connect_error_contract(&case.error);
        let dataplane = FaultingDataplane::new(case.clone());
        let world = C6World::start_with_dependencies(dataplane.clone(), healthy_identity())
            .await
            .expect("Gateway Application composition accepts the injected driven port");

        world
            .control
            .routes()
            .apply(route("public-api", "/"))
            .await
            .expect("Route command reaches the real Gateway Application owner");
        for _ in 0..128 {
            tokio::task::yield_now().await;
        }

        let report = world.handle.shutdown(Duration::from_secs(5)).await;
        let observed = dataplane.observed_errors();
        assert!(
            observed.iter().any(|actual| actual == &case.error),
            "the owner did not receive the exact injected {:?}; observed {observed:?}",
            case.error,
        );
        assert!(
            dataplane.calls().contains(&contract.operation),
            "the owner did not drive the declared {:?} operation; calls={:?}",
            contract.operation,
            dataplane.calls(),
        );

        if contract.operation == GatewayDataplaneFaultOperation::CleanupAll {
            let residual = report
                .residual_connect
                .as_ref()
                .expect("cleanup failure remains typed in the shutdown residual");
            assert!(matches!(
                residual.sweep_failures.as_slice(),
                [overdrive_gateway::application::GatewayConnectSweepFailure::CleanupAll(error)]
                    if error == &case.error
            ));
        }
    }
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER GatewayConnectDataplane redacted status projection"]
async fn every_gateway_connect_dataplane_error_collapses_to_redacted_public_status() {
    for case in gateway_connect_error_cases() {
        let contract = gateway_connect_error_contract(&case.error);
        let dataplane = FaultingDataplane::new(case.clone());
        let world = C6World::start_with_dependencies(dataplane.clone(), healthy_identity())
            .await
            .expect("Gateway Application composition accepts the injected driven port");
        world
            .control
            .routes()
            .apply(route("public-api", "/"))
            .await
            .expect("Route command reaches the real Gateway Application owner");
        for _ in 0..128 {
            tokio::task::yield_now().await;
        }

        let mut handle = Some(world.handle);
        if matches!(
            contract.operation,
            GatewayDataplaneFaultOperation::CleanupAll
                | GatewayDataplaneFaultOperation::IntentCount
                | GatewayDataplaneFaultOperation::ReceiptCount
        ) {
            handle.take().expect("GatewayHandle exists").shutdown(Duration::from_secs(5)).await;
        }

        let status = world
            .control
            .application_status()
            .await
            .expect("redacted Application status read")
            .expect("Gateway Application publishes a status row for the fault");
        assert_gateway_connect_status_redacted(&status, &case, dataplane.as_ref(), &world.key_path);

        if !matches!(
            contract.operation,
            GatewayDataplaneFaultOperation::CleanupAll
                | GatewayDataplaneFaultOperation::IntentCount
                | GatewayDataplaneFaultOperation::ReceiptCount
        ) {
            handle.take().expect("GatewayHandle exists").shutdown(Duration::from_secs(5)).await;
        }
    }
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER Gateway identity lifecycle owner error matrix"]
async fn every_gateway_identity_lifecycle_error_crosses_its_real_owner_boundary() {
    for case in gateway_identity_error_cases() {
        let contract = gateway_identity_error_contract(&case.error);
        let identity = FaultingIdentity::new(case.clone());
        let world =
            C6World::start_with_dependencies(Arc::new(NoBackendDataplane), identity.clone())
                .await
                .expect("Gateway Application composition accepts the injected identity port");
        world
            .control
            .routes()
            .apply(route("public-api", "/"))
            .await
            .expect("Route command reaches the real Gateway Application owner");
        for _ in 0..128 {
            tokio::task::yield_now().await;
        }
        let report = world.handle.shutdown(Duration::from_secs(5)).await;
        assert_eq!(identity.observed_errors(), vec![case.error.clone()]);

        if contract.operation == GatewayIdentityFaultOperation::DisableAfterDrain {
            assert_eq!(report.identity_error, Some(case.error));
        }
    }
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER Gateway identity redacted status projection"]
async fn every_gateway_identity_lifecycle_error_collapses_to_redacted_public_status() {
    for case in gateway_identity_error_cases() {
        let contract = gateway_identity_error_contract(&case.error);
        let identity = FaultingIdentity::new(case.clone());
        let world =
            C6World::start_with_dependencies(Arc::new(NoBackendDataplane), identity.clone())
                .await
                .expect("Gateway Application composition accepts the injected identity port");
        world
            .control
            .routes()
            .apply(route("public-api", "/"))
            .await
            .expect("Route command reaches the real Gateway Application owner");
        for _ in 0..128 {
            tokio::task::yield_now().await;
        }

        let mut handle = Some(world.handle);
        if contract.operation == GatewayIdentityFaultOperation::DisableAfterDrain {
            handle.take().expect("GatewayHandle exists").shutdown(Duration::from_secs(5)).await;
        }
        let status = world
            .control
            .application_status()
            .await
            .expect("redacted Application status read")
            .expect("Gateway Application publishes a status row for the fault");
        assert_gateway_identity_status_redacted(&status, &case, &world.key_path);

        if contract.operation == GatewayIdentityFaultOperation::EnsureCurrent {
            handle.take().expect("GatewayHandle exists").shutdown(Duration::from_secs(5)).await;
        }
    }
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER Route owner and real-store step"]
#[allow(
    clippy::drop_non_drop,
    reason = "GatewayControl becomes a sender-backed owner handle in DELIVER; explicit drop precedes same-redb reopen"
)]
async fn declare_replay_replace_and_withdraw_are_one_atomic_route_lifecycle() {
    let World { root: _root, intent_path, intent, control, handle, .. } =
        World::start().await.expect("gateway starts");
    let routes = control.routes();
    let key = b"public-ingress/route-set";
    let initially_empty = intent
        .get(key)
        .await
        .expect("read initial empty Route Set")
        .expect("the empty Route Set key is persisted at owner start");

    assert_eq!(
        routes.apply(route("public-api", "/")).await.expect("declare"),
        RouteApplyOutcome::Declared,
    );
    assert_eq!(
        routes.apply(route("public-api", "/")).await.expect("replay"),
        RouteApplyOutcome::Unchanged,
    );
    assert_eq!(
        routes.apply(route("public-api", "/v2")).await.expect("replace"),
        RouteApplyOutcome::Replaced,
    );
    assert_eq!(
        routes.withdraw(RouteId::new("public-api").expect("Route ID")).await.expect("withdraw"),
        RouteWithdrawOutcome::Withdrawn,
    );
    assert_eq!(
        routes
            .withdraw(RouteId::new("public-api").expect("Route ID"))
            .await
            .expect("idempotent withdraw"),
        RouteWithdrawOutcome::Unchanged,
    );
    let after_withdrawal = intent
        .get(key)
        .await
        .expect("read withdrawn Route Set")
        .expect("withdrawal persists an empty Route Set rather than deleting its key");
    assert_eq!(after_withdrawal, initially_empty, "withdrawal restores the canonical empty set");

    let report = handle.shutdown(Duration::from_secs(5)).await;
    assert!(report.is_clean(), "Route owner shutdown must release the real store: {report:?}");
    drop(control);
    drop(intent);
    let reopened =
        LocalIntentStore::open(&intent_path).expect("reopen the same redb after owner shutdown");
    let after_reopen = reopened
        .get(key)
        .await
        .expect("read Route Set after restart")
        .expect("the empty key survives reopen/recomposition");
    assert_eq!(after_reopen, initially_empty);
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER Route conflict step"]
async fn a_competing_route_id_is_rejected_without_replacing_the_current_route() {
    let world = World::start().await.expect("gateway starts");
    let routes = world.control.routes();
    routes.apply(route("public-api", "/")).await.expect("first Route");

    let error = routes.apply(route("other-api", "/")).await.expect_err("slot conflict");
    assert!(matches!(
        error,
        PublicRouteSetError::OccupiedByDifferentRoute { current, attempted }
            if current.as_str() == "public-api" && attempted.as_str() == "other-api"
    ));
    assert_eq!(
        routes.apply(route("public-api", "/")).await.expect("original remains"),
        RouteApplyOutcome::Unchanged,
    );
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER Route conflict step"]
async fn withdrawal_by_a_different_route_id_has_an_empty_state_delta() {
    let world = World::start().await.expect("gateway starts");
    let routes = world.control.routes();
    routes.apply(route("public-api", "/")).await.expect("first Route");
    let error = routes
        .withdraw(RouteId::new("other-api").expect("Route ID"))
        .await
        .expect_err("mismatched withdrawal");
    assert!(matches!(
        error,
        PublicRouteSetError::WithdrawIdMismatch { current, attempted }
            if current.as_str() == "public-api" && attempted.as_str() == "other-api"
    ));
    assert_eq!(
        routes.apply(route("public-api", "/")).await.expect("original remains"),
        RouteApplyOutcome::Unchanged,
    );
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER concurrent Route owner step"]
async fn concurrent_route_claims_have_one_winner_and_one_conflict() {
    let world = World::start().await.expect("gateway starts");
    let first = world.control.routes().clone();
    let second = first.clone();
    let a = tokio::spawn(async move { first.apply(route("route-a", "/a")).await });
    let b = tokio::spawn(async move { second.apply(route("route-b", "/b")).await });
    let outcomes = [a.await.expect("join A"), b.await.expect("join B")];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Ok(RouteApplyOutcome::Declared)))
            .count(),
        1,
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(
                outcome,
                Err(PublicRouteSetError::OccupiedByDifferentRoute { .. })
            ))
            .count(),
        1,
    );
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER gateway status redaction step"]
async fn operator_status_contains_no_manual_path_or_credential_material() {
    let world = World::start().await.expect("gateway starts");
    let application = world.control.application_status().await.expect("application status");
    let key = world.control.certified_key_status().await.expect("key status");
    let rendered = format!("{application:?}{key:?}");
    for forbidden in ["api-origin-key.pem", "PRIVATE KEY", "ciphertext", "nonce", "salt"] {
        assert!(!rendered.contains(forbidden), "status leaked {forbidden}");
    }
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER owned gateway shutdown step"]
async fn clean_shutdown_leaves_identity_empty_and_no_connect_residue() {
    let world = World::start().await.expect("gateway starts");
    let report = world.handle.shutdown(Duration::from_secs(5)).await;
    assert!(report.is_clean(), "{report:?}");
    assert!(report.identity_empty);
    assert!(report.residual_connect.is_none());
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER owned gateway shutdown identity-error mapping"]
async fn identity_disable_fault_is_retained_in_the_gateway_shutdown_report() {
    let world =
        World::start_with_identity_disable_error(Some(GatewayIdentityLifecycleError::DropFailed))
            .await
            .expect("gateway starts before the injected shutdown fault");
    let status_before =
        world.control.certified_key_status().await.expect("custody status before shutdown");
    let report = world.handle.shutdown(Duration::from_secs(5)).await;
    assert_eq!(report.identity_error, Some(GatewayIdentityLifecycleError::DropFailed));
    assert!(!report.identity_empty);
    assert!(!report.is_clean());
    assert!(report.residual_connect.is_none());
    assert_eq!(
        world
            .control
            .certified_key_status()
            .await
            .expect("custody status remains readable after identity fault"),
        status_before,
        "identity shutdown failure cannot rewrite Public Certified-Key custody status",
    );
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER manual certified-key source error step"]
async fn unreadable_manual_certified_key_refuses_boot_without_creating_route_state() {
    let root = tempfile::tempdir().expect("isolated gateway root");
    let config = GatewayConfig::new(
        "127.0.0.1".parse().expect("IPv4"),
        ManualCertifiedKeyConfig::new(
            PublicCertifiedKeyId::new("api-origin").expect("key ID"),
            root.path().join("missing-chain.pem"),
            root.path().join("missing-key.pem"),
        ),
    );
    let intent = Arc::new(
        LocalIntentStore::open(root.path().join("intent.redb")).expect("real intent store"),
    );
    let before = intent
        .scan_prefix(b"public-ingress/")
        .await
        .expect("snapshot Route/custody intent before failed boot");
    let observations = Arc::new(SimObservationStore::single_peer(
        NodeId::new("gateway-node").expect("node ID"),
        55,
    ));
    let vip_view = Arc::new(SimServiceVipView::new(BTreeMap::new()));
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());

    let result = UnboundGatewayBuilder::new(
        config,
        intent.clone(),
        observations,
        vip_view,
        Arc::new(PublicCertifiedKeyAeadCodec::new(Arc::new(SimKek::for_boot()))),
        Arc::new(SocketCookieReader::new()),
        clock,
    )
    .await;
    assert!(matches!(result, Err(GatewayBootError::ManualCertifiedKey(_))));
    let after = intent
        .scan_prefix(b"public-ingress/")
        .await
        .expect("snapshot Route/custody intent after failed boot");
    assert_eq!(after, before, "unreadable manual input has an empty intent-store delta");
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER public Certified Key custody protection failure"]
async fn custody_csprng_fault_refuses_builder_without_durable_or_status_state() {
    let root = tempfile::tempdir().expect("isolated gateway root");
    let (chain, key) = write_test_cert(&root);
    let certified_key_id = PublicCertifiedKeyId::new("api-origin").expect("key ID");
    let config = GatewayConfig::new(
        "127.0.0.1".parse().expect("IPv4"),
        ManualCertifiedKeyConfig::new(certified_key_id.clone(), chain, key),
    );
    let intent = Arc::new(
        LocalIntentStore::open(root.path().join("intent.redb")).expect("real intent store"),
    );
    let before = intent
        .scan_prefix(b"public-ingress/certified-key/")
        .await
        .expect("custody state before fault");
    let observations = Arc::new(SimObservationStore::single_peer(
        NodeId::new("gateway-node").expect("node ID"),
        56,
    ));
    let codec = PublicCertifiedKeyAeadCodec::with_random_fill_for_test(
        Arc::new(SimKek::for_boot()),
        Arc::new(|_| Err(PublicCertifiedKeyAeadError::SealFailed)),
    );
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());

    let result = UnboundGatewayBuilder::new(
        config,
        intent.clone(),
        observations.clone(),
        Arc::new(SimServiceVipView::new(BTreeMap::new())),
        Arc::new(codec),
        Arc::new(SocketCookieReader::new()),
        clock,
    )
    .await;
    assert!(matches!(
        result,
        Err(GatewayBootError::CertifiedKey(PublicCertifiedKeyError::Protection(
            PublicCertifiedKeyAeadError::SealFailed
        )))
    ));
    assert_eq!(
        intent
            .scan_prefix(b"public-ingress/certified-key/")
            .await
            .expect("custody state after fault"),
        before,
        "originating CSPRNG fault cannot create protected custody state",
    );
    assert!(
        observations
            .public_certified_key_status_row(&certified_key_id)
            .await
            .expect("status read after fault")
            .is_none(),
        "failed initial custody boot cannot publish a usable status row",
    );
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER Route owner and real-store step"]
async fn withdraw_without_a_prior_route_is_unchanged_and_preserves_the_persisted_empty_set() {
    let world = World::start().await.expect("gateway starts");
    let key = b"public-ingress/route-set";
    let before = world
        .intent
        .get(key)
        .await
        .expect("read empty set")
        .expect("owner persists the empty set at startup");
    assert_eq!(
        world
            .control
            .routes()
            .withdraw(RouteId::new("public-api").expect("Route ID"))
            .await
            .expect("inverse operation from Empty is legal"),
        RouteWithdrawOutcome::Unchanged,
    );
    let after = world
        .intent
        .get(key)
        .await
        .expect("read empty set after inverse")
        .expect("empty key remains present");
    assert_eq!(after, before);
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER singleton Route cardinality step"]
async fn route_set_cardinality_zero_one_and_many_is_closed_at_one() {
    let world = World::start().await.expect("gateway starts");
    let routes = world.control.routes();
    assert_eq!(
        routes.withdraw(RouteId::new("public-api").expect("Route ID")).await.expect("empty"),
        RouteWithdrawOutcome::Unchanged,
    );
    assert_eq!(
        routes.apply(route("public-api", "/")).await.expect("singleton"),
        RouteApplyOutcome::Declared
    );
    for attempted in ["second-api", "third-api"] {
        assert!(matches!(
            routes.apply(route(attempted, "/")).await,
            Err(PublicRouteSetError::OccupiedByDifferentRoute { current, attempted: rejected })
                if current.as_str() == "public-api" && rejected.as_str() == attempted
        ));
    }
    assert_eq!(
        routes.apply(route("public-api", "/")).await.expect("original"),
        RouteApplyOutcome::Unchanged
    );
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER public Certified Key replacement owner"]
async fn valid_replacement_commits_once_and_invalid_refresh_preserves_the_current_generation() {
    let world = World::start().await.expect("gateway starts");
    let before = world.control.certified_key_status().await.expect("status").expect("current key");

    let replacement_key =
        rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("replacement key");
    let mut replacement_params = rcgen::CertificateParams::new(vec!["api.example.com".to_owned()])
        .expect("replacement params");
    replacement_params.not_before = rcgen::date_time_ymd(2020, 1, 1);
    replacement_params.not_after = rcgen::date_time_ymd(2101, 1, 1);
    let replacement_cert =
        replacement_params.self_signed(&replacement_key).expect("replacement certificate");
    let next_chain = world.root.path().join("next-chain.pem");
    let next_key = world.root.path().join("next-key.pem");
    std::fs::write(&next_chain, replacement_cert.pem()).expect("replacement chain");
    std::fs::write(&next_key, replacement_key.serialize_pem()).expect("replacement key");
    std::fs::set_permissions(&next_key, std::fs::Permissions::from_mode(0o600))
        .expect("replacement key mode");
    std::fs::rename(&next_chain, &world.chain_path).expect("atomic chain replacement");
    std::fs::rename(&next_key, &world.key_path).expect("atomic key replacement");
    world.clock.tick(Duration::from_secs(2));
    tokio::task::yield_now().await;
    let replaced =
        world.control.certified_key_status().await.expect("status").expect("replacement status");
    assert_ne!(replaced.state, before.state, "valid replacement changes the generation");

    let replay_before = world
        .intent
        .scan_prefix(b"public-ingress/certified-key/")
        .await
        .expect("record before replay");
    world.clock.tick(Duration::from_secs(2));
    tokio::task::yield_now().await;
    assert_eq!(
        world
            .intent
            .scan_prefix(b"public-ingress/certified-key/")
            .await
            .expect("record after replay"),
        replay_before,
        "unchanged file replay has an empty durable delta",
    );

    let wrong_key =
        rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("wrong key");
    std::fs::write(&world.key_path, wrong_key.serialize_pem()).expect("mismatched key refresh");
    std::fs::set_permissions(&world.key_path, std::fs::Permissions::from_mode(0o600))
        .expect("wrong key mode");
    world.clock.tick(Duration::from_secs(2));
    tokio::task::yield_now().await;
    let after_invalid =
        world.control.certified_key_status().await.expect("status").expect("retained status");
    assert_eq!(after_invalid.state, replaced.state, "invalid refresh preserves Current");
    assert_eq!(
        after_invalid.last_install_failure,
        Some(overdrive_core::public_ingress::CertifiedKeyFailure::KeyMismatch)
    );
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER public Certified Key expiry owner"]
async fn expiry_publishes_unusable_but_retains_the_protected_record() {
    let world = World::start().await.expect("gateway starts");
    let before =
        world.intent.scan_prefix(b"public-ingress/certified-key/").await.expect("protected record");
    world.clock.tick(Duration::from_secs(100 * 366 * 24 * 60 * 60));
    tokio::task::yield_now().await;
    let status =
        world.control.certified_key_status().await.expect("status").expect("expired status");
    assert!(matches!(
        status.state,
        overdrive_core::public_ingress::PublicCertifiedKeyStatusState::Unusable {
            cause: overdrive_core::public_ingress::CertifiedKeyUsabilityFailure::Expired,
            ..
        }
    ));
    assert_eq!(
        world.intent.scan_prefix(b"public-ingress/certified-key/").await.expect("retained record"),
        before,
    );
}
