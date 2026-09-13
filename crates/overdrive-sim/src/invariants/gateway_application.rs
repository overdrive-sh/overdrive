//! Pending seeded Gateway Application lifecycle invariant contracts.
//!
//! DISTILL owns these complete drivers and observable oracles. They exercise
//! real `GatewayBuilder`, `GatewayControl`, and `GatewayHandle` ownership with
//! the approved Sim ports; current production owner calls remain RED and the
//! direct tests remain ignored. `DELIVER-PIG-GATEWAY-APPLICATION-INVARIANTS`
//! activates the completed bodies and only then adds the variants to the
//! default catalogue.

#![allow(clippy::doc_markdown, reason = "exact activation key and Contract Shape metadata")]
#![allow(clippy::missing_const_for_fn, reason = "seeded case constructors build owned fixtures")]
#![expect(
    clippy::expect_used,
    reason = "DESIGN-pinned fixtures call RED production owners until activation"
)]

use std::collections::BTreeMap;
use std::num::NonZeroU16;
use std::os::unix::fs::PermissionsExt as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, ResourcesInput, ServiceV2, WorkloadIntent,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::dataplane::{Proto, ServiceFrontend};
use overdrive_core::gateway_identity::{GatewayIdentityEpoch, GatewayIdentityFacts};
use overdrive_core::id::{BackendId, CertSerial, ContentHash, NodeId, ServiceVip, SpiffeId};
use overdrive_core::public_ingress::{
    GatewayApplicationGenerationId, GatewayApplicationGenerationStatus,
    GatewayApplicationStatusRowV1, GatewayConnectIntent, GatewayConnectPathStatus,
    GatewayDemandAckOutcome, GatewayDemandPhase, GatewayFrontendDemandApply,
    GatewayFrontendDemandRevision, GatewayFrontendDemandSnapshot, GatewayFrontendDemandWake,
    GatewayIdentityStatus, GatewayListenerStatus, GatewaySelectionReceipt, PublicCertifiedKeyId,
    PublicRouteInput, Route, RouteApplyOutcome, RouteId, RouteWithdrawOutcome, ServiceKey,
    ServiceListenerReferenceInput, SocketCookie,
};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_gateway::application::{
    GatewayControl, GatewayDemandComposition, GatewayHandle, GatewayRuntimeDependencies,
    GatewayShutdownReport, UnboundGatewayBuilder,
};
use overdrive_gateway::ports::{
    GatewayConnectCleanupSweep, GatewayConnectDataplane, GatewayConnectDataplaneError,
    GatewayIdentityLifecycleControl, GatewayIdentityLifecycleError,
};
use overdrive_gateway::runtime::GatewayLimits;
use overdrive_gateway::{GatewayConfig, ManualCertifiedKeyConfig};
use overdrive_host::ca::PublicCertifiedKeyAeadCodec;
use overdrive_host::socket_cookie::SocketCookieReader;
use overdrive_store_local::LocalIntentStore;
use rand::rngs::StdRng;
use rand::{Rng as _, SeedableRng as _};
use rustls::pki_types::{CertificateDer, ServerName};
use tempfile::TempDir;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_rustls::TlsConnector;

use crate::adapters::gateway::SimGatewayClientMtls;
use crate::adapters::kek::SimKek;
use crate::adapters::observation_store::SimObservationStore;
use crate::adapters::read_ports::SimServiceVipView;
use crate::harness::{InvariantResult, InvariantStatus};

const GENERATION_NAME: &str = "gateway-application-generation-lifecycle-is-safe";
const SHUTDOWN_NAME: &str = "gateway-application-shutdown-converges";
const HOST: &str = "host-0";
const PUBLIC_HOSTNAME: &str = "api.example.com";
const PUBLIC_CERTIFIED_KEY_ID: &str = "api-origin";
const SERVICE_NAME: &str = "api";
const POLL_BOUND: usize = 2_048;

pub(crate) async fn evaluate_generation_lifecycle_is_safe(seed: u64) -> InvariantResult {
    let case = GatewayApplicationInvariantCase::generation_lifecycle(seed);
    let material = PublicCertificateFixture::mint();
    let first = drive_generation_lifecycle(case.clone(), material.clone()).await;
    let second = drive_generation_lifecycle(case, material).await;
    invariant_result(GENERATION_NAME, seed, first, second)
}

pub(crate) async fn evaluate_shutdown_converges(seed: u64) -> InvariantResult {
    let case = GatewayApplicationInvariantCase::shutdown(seed);
    let material = PublicCertificateFixture::mint();
    let first = drive_shutdown(case.clone(), material.clone()).await;
    let second = drive_shutdown(case, material).await;
    invariant_result(SHUTDOWN_NAME, seed, first, second)
}

#[derive(Debug, Clone)]
pub(crate) enum GatewayApplicationSimInput {
    ApplyRoute(Route),
    WithdrawRoute(RouteId),
    RetainCurrentPublicConnection,
    ReleaseRetainedPublicConnection,
    RestartOwner,
    BeginShutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GatewayApplicationSimFault {
    HoldDemandCompletion(GatewayFrontendDemandApply),
    ReleaseDemandCompletion(GatewayFrontendDemandApply),
}

#[derive(Debug, Clone)]
pub(crate) struct GatewayApplicationInvariantCase {
    pub(crate) inputs: Vec<GatewayApplicationSimInput>,
    pub(crate) faults: Vec<GatewayApplicationSimFault>,
}

impl GatewayApplicationInvariantCase {
    pub(crate) fn generation_lifecycle(seed: u64) -> Self {
        let held = demand_apply(4, 8081);
        let mut rng = StdRng::seed_from_u64(seed);
        let mut inputs = vec![
            GatewayApplicationSimInput::ApplyRoute(route("/v1", 8080)),
            GatewayApplicationSimInput::RetainCurrentPublicConnection,
            GatewayApplicationSimInput::ApplyRoute(route("/v2", 8081)),
            GatewayApplicationSimInput::ApplyRoute(route("/v3", 8082)),
        ];
        let withdraw = GatewayApplicationSimInput::WithdrawRoute(
            RouteId::new("public-api").expect("DESIGN-valid Route ID"),
        );
        // Restart only after the retained public connection has been released.
        // The seed selects whether restart relists the third Route or relists
        // the already-withdrawn Empty set; both are legal durable-intent
        // boundaries and neither compares process-local demand revisions.
        if rng.gen_bool(0.5) {
            inputs.extend([
                withdraw,
                GatewayApplicationSimInput::ReleaseRetainedPublicConnection,
                GatewayApplicationSimInput::RestartOwner,
            ]);
        } else {
            inputs.extend([
                GatewayApplicationSimInput::ReleaseRetainedPublicConnection,
                GatewayApplicationSimInput::RestartOwner,
                withdraw,
            ]);
        }
        Self {
            inputs,
            faults: vec![
                GatewayApplicationSimFault::HoldDemandCompletion(held),
                GatewayApplicationSimFault::ReleaseDemandCompletion(held),
            ],
        }
    }

    pub(crate) fn shutdown(seed: u64) -> Self {
        let held = demand_apply(4, 8081);
        let inputs = vec![
            GatewayApplicationSimInput::ApplyRoute(route("/v1", 8080)),
            GatewayApplicationSimInput::RetainCurrentPublicConnection,
            GatewayApplicationSimInput::ApplyRoute(route("/v2", 8081)),
            GatewayApplicationSimInput::BeginShutdown,
            GatewayApplicationSimInput::ReleaseRetainedPublicConnection,
        ];
        let mut rng = StdRng::seed_from_u64(seed);
        let hold_fault = GatewayApplicationSimFault::HoldDemandCompletion(held);
        let release_fault = GatewayApplicationSimFault::ReleaseDemandCompletion(held);
        let faults = if rng.gen_bool(0.5) {
            vec![release_fault, hold_fault]
        } else {
            vec![hold_fault, release_fault]
        };
        Self { inputs, faults }
    }
}

fn route(path: &str, port: u16) -> Route {
    Route::from_submit(PublicRouteInput {
        id: "public-api".to_owned(),
        hostname: "api.example.com".to_owned(),
        path: path.to_owned(),
        path_match: "segment_prefix".to_owned(),
        certified_key: "api-origin".to_owned(),
        target: ServiceListenerReferenceInput {
            service: "api".to_owned(),
            port,
            protocol: "tcp".to_owned(),
        },
    })
    .expect("DESIGN-valid Route fixture")
}

fn demand_apply(revision: u64, port: u16) -> GatewayFrontendDemandApply {
    let frontend = ServiceFrontend::new(
        ServiceVip::new("10.96.0.8".parse().expect("IPv4")).expect("Service VIP"),
        NonZeroU16::new(port).expect("nonzero listener"),
        Proto::Tcp,
    )
    .expect("IPv4 TCP Service Frontend");
    GatewayFrontendDemandApply {
        revision: GatewayFrontendDemandRevision::new(revision).expect("nonzero revision"),
        service_key: ServiceKey::from_frontend(frontend),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApplicationObservation {
    listener: GatewayListenerStatus,
    staged: Option<GatewayApplicationGenerationStatus>,
    current: Option<GatewayApplicationGenerationStatus>,
    draining: Vec<GatewayApplicationGenerationStatus>,
    unavailable: Option<overdrive_core::public_ingress::GatewayApplicationUnavailableCause>,
    gateway_identity: GatewayIdentityStatus,
    connect_path: GatewayConnectPathStatus,
}

impl From<GatewayApplicationStatusRowV1> for ApplicationObservation {
    fn from(row: GatewayApplicationStatusRowV1) -> Self {
        Self {
            listener: row.listener,
            staged: row.staged,
            current: row.current,
            draining: row.draining,
            unavailable: row.unavailable,
            gateway_identity: row.gateway_identity,
            connect_path: row.connect_path,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShutdownObservation {
    clean: bool,
    forced_connections: u32,
    task_failures: Vec<overdrive_gateway::application::GatewayTaskFailure>,
    cleanup_failures: Vec<overdrive_gateway::application::GatewayCleanupFailure>,
    application_error_count: usize,
    identity_error: bool,
    identity_empty: bool,
    residual_connect: Option<(Option<u32>, Option<u32>, u32)>,
}

impl From<&GatewayShutdownReport> for ShutdownObservation {
    fn from(report: &GatewayShutdownReport) -> Self {
        Self {
            clean: report.is_clean(),
            forced_connections: report.forced_connections,
            task_failures: report.task_failures.clone(),
            cleanup_failures: report.cleanup_failures.clone(),
            application_error_count: report.application_errors.len(),
            identity_error: report.identity_error.is_some(),
            identity_empty: report.identity_empty,
            residual_connect: report.residual_connect.as_ref().map(|residual| {
                (residual.live_intents, residual.live_receipts, residual.ledger_entries)
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GatewayObservation {
    Owner {
        label: &'static str,
        application: Option<Box<ApplicationObservation>>,
        demand: GatewayFrontendDemandSnapshot,
        identity: Option<GatewayIdentityFacts>,
    },
    DemandCompletion {
        apply: GatewayFrontendDemandApply,
        outcome: GatewayDemandAckOutcome,
    },
    Shutdown(ShutdownObservation),
}

#[derive(Clone)]
struct PublicCertificateFixture {
    certificate_pem: String,
    private_key_pem: String,
    certificate_der: CertificateDer<'static>,
}

impl PublicCertificateFixture {
    fn mint() -> Self {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("seed-independent public test key");
        let mut params = rcgen::CertificateParams::new(vec![PUBLIC_HOSTNAME.to_owned()])
            .expect("public certificate params");
        params.is_ca = rcgen::IsCa::NoCa;
        params.not_before = rcgen::date_time_ymd(2020, 1, 1);
        params.not_after = rcgen::date_time_ymd(2040, 1, 1);
        let certificate = params.self_signed(&key).expect("public test certificate");
        Self {
            certificate_pem: certificate.pem(),
            private_key_pem: key.serialize_pem(),
            certificate_der: certificate.der().clone(),
        }
    }
}

#[derive(Default)]
struct ScenarioDataplane {
    intents: Mutex<BTreeMap<SocketCookie, GatewayConnectIntent>>,
    selected_identity: Mutex<Option<SpiffeId>>,
}

impl ScenarioDataplane {
    fn with_selected_identity(identity: SpiffeId) -> Self {
        Self { intents: Mutex::new(BTreeMap::new()), selected_identity: Mutex::new(Some(identity)) }
    }
}

#[async_trait]
impl GatewayConnectDataplane for ScenarioDataplane {
    async fn probe(
        &self,
        _deadline: Instant,
        _clock: &dyn Clock,
    ) -> Result<(), GatewayConnectDataplaneError> {
        Ok(())
    }

    fn register(&self, intent: GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError> {
        let mut intents = self.intents.lock().expect("scenario intent lock");
        if intents.insert(intent.socket_cookie, intent).is_some() {
            return Err(GatewayConnectDataplaneError::IntentAlreadyRegistered);
        }
        drop(intents);
        Ok(())
    }

    fn take_receipt(
        &self,
        intent: &GatewayConnectIntent,
    ) -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError> {
        let intents = self.intents.lock().expect("scenario intent lock");
        let Some(registered) = intents.get(&intent.socket_cookie) else {
            return Err(GatewayConnectDataplaneError::ReceiptMissing);
        };
        if registered != intent {
            return Err(GatewayConnectDataplaneError::ReceiptMismatch);
        }
        drop(intents);
        Ok(GatewaySelectionReceipt::Selected {
            socket_cookie: intent.socket_cookie,
            service_key: intent.service_key,
            backend_id: BackendId::new(1).expect("scenario BackendId"),
        })
    }

    fn cleanup(&self, intent: &GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError> {
        self.intents.lock().expect("scenario intent lock").remove(&intent.socket_cookie);
        Ok(())
    }

    fn cleanup_all_gateway_intents(
        &self,
    ) -> Result<GatewayConnectCleanupSweep, GatewayConnectDataplaneError> {
        let mut intents = self.intents.lock().expect("scenario intent lock");
        let removed_intents = u32::try_from(intents.len()).unwrap_or(u32::MAX);
        intents.clear();
        drop(intents);
        Ok(GatewayConnectCleanupSweep { removed_intents, removed_receipts: 0 })
    }

    fn selected_backend_identity(
        &self,
        backend_id: BackendId,
    ) -> Result<SpiffeId, GatewayConnectDataplaneError> {
        self.selected_identity
            .lock()
            .expect("selected identity lock")
            .clone()
            .ok_or(GatewayConnectDataplaneError::BackendIdentityMissing { backend_id })
    }

    fn live_intent_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
        Ok(u32::try_from(self.intents.lock().expect("scenario intent lock").len())
            .unwrap_or(u32::MAX))
    }

    fn live_receipt_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
        Ok(0)
    }
}

struct ScenarioIdentityLifecycle {
    current: Mutex<Option<GatewayIdentityFacts>>,
    sender: watch::Sender<Option<GatewayIdentityFacts>>,
}

impl ScenarioIdentityLifecycle {
    fn current_at_start() -> Arc<Self> {
        let facts = GatewayIdentityFacts {
            epoch: GatewayIdentityEpoch::first(),
            spiffe_id: SpiffeId::new("spiffe://overdrive.local/gateway/gateway-node")
                .expect("gateway SPIFFE ID"),
            serial: CertSerial::new("01").expect("gateway serial"),
            not_after: overdrive_core::UnixInstant::from_unix_duration(Duration::from_secs(
                2_208_988_800,
            )),
        };
        let (sender, _) = watch::channel(Some(facts.clone()));
        Arc::new(Self { current: Mutex::new(Some(facts)), sender })
    }
}

#[async_trait]
impl GatewayIdentityLifecycleControl for ScenarioIdentityLifecycle {
    async fn ensure_current(
        &self,
        _deadline: Instant,
    ) -> Result<GatewayIdentityFacts, GatewayIdentityLifecycleError> {
        self.current
            .lock()
            .expect("scenario identity lock")
            .clone()
            .ok_or(GatewayIdentityLifecycleError::Closed)
    }

    fn current(&self) -> Option<GatewayIdentityFacts> {
        self.current.lock().expect("scenario identity lock").clone()
    }

    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>> {
        self.sender.subscribe()
    }

    async fn disable_after_drain(
        &self,
        _deadline: Instant,
    ) -> Result<(), GatewayIdentityLifecycleError> {
        self.current.lock().expect("scenario identity lock").take();
        self.sender.send_replace(None);
        Ok(())
    }
}

struct NoopDemandWake;
impl GatewayFrontendDemandWake for NoopDemandWake {
    fn wake(&self, _service_id: overdrive_core::id::ServiceId) {}
}

struct ScenarioWorld {
    _root: TempDir,
    chain_path: std::path::PathBuf,
    key_path: std::path::PathBuf,
    certificate_der: CertificateDer<'static>,
    intent: Arc<LocalIntentStore>,
    observations: Arc<SimObservationStore>,
    vip_view: Arc<SimServiceVipView>,
    clock: Arc<crate::adapters::clock::SimClock>,
    dataplane: Arc<ScenarioDataplane>,
    upstream_tasks: Vec<tokio::task::JoinHandle<()>>,
    handle: Option<GatewayHandle>,
    control: Option<GatewayControl>,
    demand: Option<GatewayDemandComposition>,
    identity: Option<Arc<ScenarioIdentityLifecycle>>,
}

impl ScenarioWorld {
    async fn start(material: PublicCertificateFixture) -> Result<Self, String> {
        let root = tempfile::tempdir().map_err(|error| format!("tempdir: {error}"))?;
        let chain_path = root.path().join("public-chain.pem");
        let key_path = root.path().join("public-key.pem");
        std::fs::write(&chain_path, &material.certificate_pem)
            .map_err(|error| format!("write chain: {error}"))?;
        std::fs::write(&key_path, &material.private_key_pem)
            .map_err(|error| format!("write key: {error}"))?;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("key mode: {error}"))?;

        let intent = Arc::new(
            LocalIntentStore::open(root.path().join("intent.redb"))
                .map_err(|error| format!("intent store: {error}"))?,
        );
        let vip = ServiceVip::new("127.0.0.1".parse().expect("IPv4"))
            .map_err(|error| format!("Service VIP: {error}"))?;
        let service = ServiceV2::from_submit(ServiceSpecInput {
            id: SERVICE_NAME.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 10, memory_bytes: 16 * 1024 * 1024 },
            driver: DriverInput::Exec(ExecInput {
                command: "/bin/true".to_owned(),
                args: Vec::new(),
            }),
            listeners: [8080_u16, 8081, 8082]
                .into_iter()
                .map(|port| ListenerInput { port, protocol: "tcp".to_owned() })
                .collect(),
            startup_probes: Vec::new(),
            readiness_probes: Vec::new(),
            liveness_probes: Vec::new(),
        })
        .map_err(|error| format!("Service input: {error}"))?;
        let workload_id = service.id.clone();
        let aggregate = WorkloadIntent::Service(service);
        let digest = aggregate.spec_digest().map_err(|error| format!("Service digest: {error}"))?;
        let archived =
            aggregate.archive_for_store().map_err(|error| format!("Service archive: {error}"))?;
        intent
            .put(IntentKey::for_workload(&workload_id).as_bytes(), archived.as_ref())
            .await
            .map_err(|error| format!("persist Service: {error}"))?;

        let vip_view = Arc::new(SimServiceVipView::new(BTreeMap::<ContentHash, ServiceVip>::from(
            [(digest, vip)],
        )));
        let observations = Arc::new(SimObservationStore::single_peer(
            NodeId::new("gateway-node").map_err(|error| format!("node ID: {error}"))?,
            54_104,
        ));
        let clock = Arc::new(crate::adapters::clock::SimClock::new());
        let selected_identity = SpiffeId::new("spiffe://overdrive.local/workload/api/alloc/api-0")
            .map_err(|error| format!("selected identity: {error}"))?;
        let dataplane = Arc::new(ScenarioDataplane::with_selected_identity(selected_identity));

        let mut upstream_tasks = Vec::new();
        for port in [8080_u16, 8081, 8082] {
            let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
                .await
                .map_err(|error| format!("bind scenario upstream {port}: {error}"))?;
            upstream_tasks.push(tokio::spawn(async move {
                while let Ok((socket, _)) = listener.accept().await {
                    drop(socket);
                }
            }));
        }

        let mut world = Self {
            _root: root,
            chain_path,
            key_path,
            certificate_der: material.certificate_der,
            intent,
            observations,
            vip_view,
            clock,
            dataplane,
            upstream_tasks,
            handle: None,
            control: None,
            demand: None,
            identity: None,
        };
        world.compose_owner().await?;
        Ok(world)
    }

    async fn compose_owner(&mut self) -> Result<(), String> {
        let config = GatewayConfig::new(
            std::net::Ipv4Addr::LOCALHOST,
            ManualCertifiedKeyConfig::new(
                PublicCertifiedKeyId::new(PUBLIC_CERTIFIED_KEY_ID)
                    .map_err(|error| format!("certified-key ID: {error}"))?,
                self.chain_path.clone(),
                self.key_path.clone(),
            ),
        );
        let clock: Arc<dyn Clock> = self.clock.clone();
        let intent: Arc<dyn IntentStore> = self.intent.clone();
        let observations: Arc<dyn ObservationStore> = self.observations.clone();
        let builder = UnboundGatewayBuilder::new(
            config,
            intent,
            observations,
            self.vip_view.clone(),
            Arc::new(PublicCertifiedKeyAeadCodec::new(Arc::new(SimKek::for_boot()))),
            Arc::new(SocketCookieReader::new()),
            clock.clone(),
        )
        .await
        .map_err(|error| format!("GatewayBuilder::new: {error}"))?;
        let (builder, demand, seal) = builder.bind_demand_wake(Arc::new(NoopDemandWake));
        let identity = ScenarioIdentityLifecycle::current_at_start();
        let mtls = Arc::new(SimGatewayClientMtls::new(seal, clock));
        mtls.set_peer(
            self.dataplane
                .selected_identity
                .lock()
                .expect("selected identity lock")
                .clone()
                .expect("scenario selected identity"),
        );
        let dependencies = GatewayRuntimeDependencies::new(
            GatewayLimits::first_slice(),
            self.dataplane.clone(),
            mtls,
            identity.clone(),
        );
        let (handle, control) = builder.start(dependencies).await.into_parts();
        self.handle = Some(handle);
        self.control = Some(control);
        self.demand = Some(demand);
        self.identity = Some(identity);
        Ok(())
    }

    fn control(&self) -> &GatewayControl {
        self.control.as_ref().expect("composed GatewayControl")
    }

    fn demand(&self) -> &GatewayDemandComposition {
        self.demand.as_ref().expect("composed demand")
    }

    fn identity(&self) -> &ScenarioIdentityLifecycle {
        self.identity.as_deref().expect("composed identity")
    }

    async fn observe(
        &self,
        label: &'static str,
        trace: &mut Vec<GatewayObservation>,
    ) -> Result<(), String> {
        let application = self
            .control()
            .application_status()
            .await
            .map_err(|error| format!("application status: {error}"))?
            .map(ApplicationObservation::from)
            .map(Box::new);
        let demand = self.demand().read().snapshot().as_ref().clone();
        assert_status_demand_coherent(application.as_deref(), &demand)?;
        trace.push(GatewayObservation::Owner {
            label,
            application,
            demand,
            identity: self.identity().current(),
        });
        Ok(())
    }

    async fn wait_for_staged(&self) -> Result<GatewayFrontendDemandApply, String> {
        for _ in 0..POLL_BOUND {
            let status = self
                .control()
                .application_status()
                .await
                .map_err(|error| format!("application status: {error}"))?;
            let snapshot = self.demand().read().snapshot();
            if let Some(staged) = status.as_ref().and_then(|row| row.staged.as_ref())
                && snapshot.entries.iter().any(|entry| {
                    entry.application_generation == staged.application_generation
                        && entry.service_key == staged.service_key
                        && entry.phase == GatewayDemandPhase::Staged
                })
            {
                return Ok(GatewayFrontendDemandApply {
                    revision: staged.demand_revision,
                    service_key: staged.service_key,
                });
            }
            tokio::task::yield_now().await;
        }
        Err("Gateway Application did not expose Staged demand within the poll bound".to_owned())
    }

    async fn wait_for_current(
        &self,
        generation: GatewayApplicationGenerationId,
    ) -> Result<(), String> {
        for _ in 0..POLL_BOUND {
            let status = self
                .control()
                .application_status()
                .await
                .map_err(|error| format!("application status: {error}"))?;
            if status.as_ref().is_some_and(|row| {
                row.staged.is_none()
                    && row.current.as_ref().is_some_and(|current| {
                        current.application_generation == generation
                            && matches!(row.listener, GatewayListenerStatus::Bound { .. })
                    })
            }) {
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
        Err("Gateway Application did not promote the acknowledged generation".to_owned())
    }

    async fn wait_for_no_current(&self, require_empty_demand: bool) -> Result<(), String> {
        for _ in 0..POLL_BOUND {
            let status = self
                .control()
                .application_status()
                .await
                .map_err(|error| format!("application status: {error}"))?;
            let demand_empty = self.demand().read().snapshot().entries.is_empty();
            if status.as_ref().is_some_and(|row| {
                row.staged.is_none()
                    && row.current.is_none()
                    && matches!(row.listener, GatewayListenerStatus::Unbound)
                    && (!require_empty_demand || row.draining.is_empty())
            }) && (!require_empty_demand || demand_empty)
            {
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
        Err("Gateway Application did not converge to the requested empty state".to_owned())
    }

    fn acknowledge(
        &self,
        apply: GatewayFrontendDemandApply,
        trace: &mut Vec<GatewayObservation>,
    ) -> Result<GatewayDemandAckOutcome, String> {
        let dispatch = self
            .demand()
            .dispatch_ports()
            .ok_or_else(|| "enabled Gateway demand lacks dispatch ports".to_owned())?;
        let outcome = dispatch.acknowledge().applied(apply);
        trace.push(GatewayObservation::DemandCompletion { apply, outcome });
        Ok(outcome)
    }

    async fn retain_public_connection(&self) -> Result<RetainedPublicConnection, String> {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(self.certificate_der.clone()).map_err(|error| format!("public root: {error}"))?;
        let config =
            rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
        let stream = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, 443))
            .await
            .map_err(|error| format!("public TCP connection: {error}"))?;
        let server_name = ServerName::try_from(PUBLIC_HOSTNAME.to_owned())
            .map_err(|error| format!("public server name: {error}"))?;
        let tls = tokio::time::timeout(
            Duration::from_secs(2),
            TlsConnector::from(Arc::new(config)).connect(server_name, stream),
        )
        .await
        .map_err(|_| "public TLS handshake exceeded fixture bound".to_owned())?
        .map_err(|error| format!("public TLS handshake: {error}"))?;
        Ok(RetainedPublicConnection { _tls: tls })
    }

    async fn restart_owner(
        &mut self,
        route_present: bool,
        trace: &mut Vec<GatewayObservation>,
    ) -> Result<(), String> {
        let report = self
            .handle
            .take()
            .expect("composed GatewayHandle")
            .shutdown(Duration::from_secs(5))
            .await;
        if !report.is_clean() {
            return Err(format!("restart shutdown was not clean: {report:?}"));
        }
        trace.push(GatewayObservation::Shutdown(ShutdownObservation::from(&report)));
        self.control.take();
        self.demand.take();
        self.identity.take();
        self.compose_owner().await?;
        if route_present {
            let staged = self.wait_for_staged().await?;
            let generation = self
                .current_staged_generation()
                .await?
                .ok_or_else(|| "relisted Route did not expose Staged".to_owned())?;
            if self.acknowledge(staged, trace)? != GatewayDemandAckOutcome::Applied {
                return Err("relisted Route demand acknowledgment was stale".to_owned());
            }
            self.wait_for_current(generation).await?;
        } else {
            self.wait_for_no_current(true).await?;
        }
        self.observe("after-restart", trace).await
    }

    async fn current_staged_generation(
        &self,
    ) -> Result<Option<GatewayApplicationGenerationId>, String> {
        Ok(self
            .control()
            .application_status()
            .await
            .map_err(|error| format!("application status: {error}"))?
            .and_then(|status| status.staged.map(|staged| staged.application_generation)))
    }

    async fn finish(mut self) -> Result<GatewayShutdownReport, String> {
        let report = self
            .handle
            .take()
            .expect("composed GatewayHandle")
            .shutdown(Duration::from_secs(5))
            .await;
        for task in self.upstream_tasks.drain(..) {
            task.abort();
            let _ = task.await;
        }
        Ok(report)
    }
}

struct RetainedPublicConnection {
    _tls: tokio_rustls::client::TlsStream<TcpStream>,
}

fn assert_status_demand_coherent(
    application: Option<&ApplicationObservation>,
    demand: &GatewayFrontendDemandSnapshot,
) -> Result<(), String> {
    let mut expected = BTreeMap::new();
    if let Some(application) = application {
        if let Some(staged) = &application.staged {
            expected.insert(
                staged.application_generation,
                (staged.service_key, GatewayDemandPhase::Staged),
            );
        }
        if let Some(current) = &application.current {
            expected.insert(
                current.application_generation,
                (current.service_key, GatewayDemandPhase::Current),
            );
        }
        for draining in &application.draining {
            expected.insert(
                draining.application_generation,
                (draining.service_key, GatewayDemandPhase::Draining),
            );
        }
    }
    let mut actual = BTreeMap::new();
    for entry in &demand.entries {
        if ServiceKey::from_frontend(entry.frontend) != entry.service_key {
            return Err("demand entry ServiceKey does not match its public frontend".to_owned());
        }
        if actual.insert(entry.application_generation, (entry.service_key, entry.phase)).is_some() {
            return Err("demand snapshot contains duplicate application generation".to_owned());
        }
    }
    if actual != expected {
        return Err(format!(
            "status/demand phase-precedence union diverged: expected={expected:?} actual={actual:?}"
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_lines, reason = "one exact ordered owner-input schedule and oracle")]
async fn drive_generation_lifecycle(
    case: GatewayApplicationInvariantCase,
    material: PublicCertificateFixture,
) -> Result<Vec<GatewayObservation>, String> {
    let held = case
        .faults
        .iter()
        .find_map(|fault| match fault {
            GatewayApplicationSimFault::HoldDemandCompletion(apply) => Some(*apply),
            GatewayApplicationSimFault::ReleaseDemandCompletion(_) => None,
        })
        .ok_or_else(|| "generation case lacks HoldDemandCompletion".to_owned())?;
    if !case
        .faults
        .iter()
        .any(|fault| matches!(fault, GatewayApplicationSimFault::ReleaseDemandCompletion(apply) if *apply == held))
    {
        return Err("generation case does not release the exact held demand apply".to_owned());
    }

    let mut world = ScenarioWorld::start(material).await?;
    let mut trace = Vec::new();
    let mut retained: Option<RetainedPublicConnection> = None;
    let mut route_present = false;
    let mut held_observed = false;
    let mut stale_released = false;

    for input in case.inputs {
        match input {
            GatewayApplicationSimInput::ApplyRoute(route) => {
                let port = route.service_listener().port().get();
                let outcome = world
                    .control()
                    .routes()
                    .apply(route)
                    .await
                    .map_err(|error| format!("apply Route on {port}: {error}"))?;
                if !matches!(outcome, RouteApplyOutcome::Declared | RouteApplyOutcome::Replaced) {
                    return Err(format!("Route apply on {port} was not a mutation: {outcome:?}"));
                }
                route_present = true;
                let apply = world.wait_for_staged().await?;
                let generation = world
                    .current_staged_generation()
                    .await?
                    .ok_or_else(|| "staged status disappeared".to_owned())?;
                if apply == held {
                    held_observed = true;
                    world.observe("held-second-demand", &mut trace).await?;
                    continue;
                }
                if held_observed && !stale_released {
                    if world.acknowledge(held, &mut trace)? != GatewayDemandAckOutcome::Stale {
                        return Err(
                            "superseded second-generation completion was not stale".to_owned()
                        );
                    }
                    stale_released = true;
                }
                if world.acknowledge(apply, &mut trace)? != GatewayDemandAckOutcome::Applied {
                    return Err(format!("current staged demand was not applied: {apply:?}"));
                }
                world.wait_for_current(generation).await?;
                world.observe("after-route-promotion", &mut trace).await?;
            }
            GatewayApplicationSimInput::WithdrawRoute(route_id) => {
                let outcome = world
                    .control()
                    .routes()
                    .withdraw(route_id)
                    .await
                    .map_err(|error| format!("withdraw Route: {error}"))?;
                if outcome != RouteWithdrawOutcome::Withdrawn {
                    return Err(format!("Route withdrawal was not effective: {outcome:?}"));
                }
                route_present = false;
                world.wait_for_no_current(false).await?;
                world.observe("after-route-withdrawal", &mut trace).await?;
            }
            GatewayApplicationSimInput::RetainCurrentPublicConnection => {
                if retained.is_some() {
                    return Err("case retained more than one public connection".to_owned());
                }
                retained = Some(world.retain_public_connection().await?);
                world.observe("public-connection-retained", &mut trace).await?;
            }
            GatewayApplicationSimInput::ReleaseRetainedPublicConnection => {
                drop(retained.take().ok_or_else(|| {
                    "case released a public connection before retaining one".to_owned()
                })?);
                for _ in 0..POLL_BOUND {
                    let snapshot = world.demand().read().snapshot();
                    if !snapshot
                        .entries
                        .iter()
                        .any(|entry| entry.phase == GatewayDemandPhase::Draining)
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
                world.observe("public-connection-released", &mut trace).await?;
            }
            GatewayApplicationSimInput::RestartOwner => {
                world.restart_owner(route_present, &mut trace).await?;
            }
            GatewayApplicationSimInput::BeginShutdown => {
                return Err("generation-lifecycle case unexpectedly began shutdown".to_owned());
            }
        }
    }

    if !held_observed || !stale_released || retained.is_some() {
        return Err(format!(
            "generation case incomplete: held={held_observed} stale_released={stale_released} retained={} ",
            retained.is_some(),
        ));
    }
    if !route_present {
        world.wait_for_no_current(true).await?;
    }
    let report = world.finish().await?;
    if !report.is_clean() {
        return Err(format!("final generation shutdown was not clean: {report:?}"));
    }
    trace.push(GatewayObservation::Shutdown(ShutdownObservation::from(&report)));
    Ok(trace)
}

#[allow(clippy::too_many_lines, reason = "one exact shutdown race schedule and joined oracle")]
async fn drive_shutdown(
    case: GatewayApplicationInvariantCase,
    material: PublicCertificateFixture,
) -> Result<Vec<GatewayObservation>, String> {
    let held = case
        .faults
        .iter()
        .find_map(|fault| match fault {
            GatewayApplicationSimFault::HoldDemandCompletion(apply) => Some(*apply),
            GatewayApplicationSimFault::ReleaseDemandCompletion(_) => None,
        })
        .ok_or_else(|| "shutdown case lacks held completion".to_owned())?;
    if !case.faults.iter().any(
        |fault| matches!(fault, GatewayApplicationSimFault::ReleaseDemandCompletion(apply) if *apply == held),
    ) {
        return Err("shutdown case does not release the exact held completion".to_owned());
    }
    let release_completion_first = matches!(
        case.faults.first(),
        Some(GatewayApplicationSimFault::ReleaseDemandCompletion(apply)) if *apply == held
    );

    let mut world = ScenarioWorld::start(material).await?;
    let mut trace = Vec::new();
    let mut retained: Option<RetainedPublicConnection> = None;
    let mut shutdown_task = None;

    for input in case.inputs {
        match input {
            GatewayApplicationSimInput::ApplyRoute(route) => {
                world
                    .control()
                    .routes()
                    .apply(route)
                    .await
                    .map_err(|error| format!("shutdown Route apply: {error}"))?;
                let apply = world.wait_for_staged().await?;
                let generation = world
                    .current_staged_generation()
                    .await?
                    .ok_or_else(|| "shutdown case staged status disappeared".to_owned())?;
                if apply == held {
                    world.observe("shutdown-staged-held", &mut trace).await?;
                } else {
                    if world.acknowledge(apply, &mut trace)? != GatewayDemandAckOutcome::Applied {
                        return Err("shutdown initial demand was not applied".to_owned());
                    }
                    world.wait_for_current(generation).await?;
                    world.observe("shutdown-current", &mut trace).await?;
                }
            }
            GatewayApplicationSimInput::RetainCurrentPublicConnection => {
                retained = Some(world.retain_public_connection().await?);
                world.observe("shutdown-connection-retained", &mut trace).await?;
            }
            GatewayApplicationSimInput::BeginShutdown => {
                let handle = world.handle.take().expect("shutdown consumes GatewayHandle");
                shutdown_task =
                    Some(tokio::spawn(
                        async move { handle.shutdown(Duration::from_secs(5)).await },
                    ));
                tokio::task::yield_now().await;
                if release_completion_first {
                    let outcome = world.acknowledge(held, &mut trace)?;
                    if outcome != GatewayDemandAckOutcome::Stale {
                        return Err(
                            "held completion after shutdown did not become stale".to_owned()
                        );
                    }
                }
            }
            GatewayApplicationSimInput::ReleaseRetainedPublicConnection => {
                drop(retained.take().ok_or_else(|| {
                    "shutdown released a public connection before retaining one".to_owned()
                })?);
                if !release_completion_first {
                    let outcome = world.acknowledge(held, &mut trace)?;
                    if outcome != GatewayDemandAckOutcome::Stale {
                        return Err(
                            "held completion after connection release was not stale".to_owned()
                        );
                    }
                }
            }
            GatewayApplicationSimInput::WithdrawRoute(_) => {
                return Err("shutdown case unexpectedly withdrew Route".to_owned());
            }
            GatewayApplicationSimInput::RestartOwner => {
                return Err("shutdown case unexpectedly restarted owner".to_owned());
            }
        }
    }

    if retained.is_some() {
        return Err("shutdown case left its retained public connection open".to_owned());
    }
    let report = tokio::time::timeout(
        Duration::from_secs(2),
        shutdown_task.ok_or_else(|| "shutdown case never consumed GatewayHandle".to_owned())?,
    )
    .await
    .map_err(|_| "GatewayHandle shutdown did not join after releases".to_owned())?
    .map_err(|error| format!("GatewayHandle shutdown task: {error}"))?;
    for task in world.upstream_tasks.drain(..) {
        task.abort();
        let _ = task.await;
    }
    let projection = ShutdownObservation::from(&report);
    if !projection.clean
        || !projection.identity_empty
        || projection.residual_connect.is_some()
        || projection.application_error_count != 0
    {
        return Err(format!("shutdown did not converge cleanly: {report:?}"));
    }
    trace.push(GatewayObservation::Shutdown(projection));

    world.wait_for_no_current(true).await?;
    world.observe("shutdown-final", &mut trace).await?;
    if world.identity().current().is_some() {
        return Err("gateway identity remained observable after joined shutdown".to_owned());
    }
    Ok(trace)
}

fn invariant_result(
    name: &str,
    seed: u64,
    first: Result<Vec<GatewayObservation>, String>,
    second: Result<Vec<GatewayObservation>, String>,
) -> InvariantResult {
    match (first, second) {
        (Ok(first), Ok(second)) if first == second => InvariantResult {
            name: name.to_owned(),
            status: InvariantStatus::Pass,
            tick: u64::try_from(first.len()).unwrap_or(u64::MAX),
            host: HOST.to_owned(),
            cause: None,
        },
        (Ok(first), Ok(second)) => InvariantResult {
            name: name.to_owned(),
            status: InvariantStatus::Fail,
            tick: 0,
            host: HOST.to_owned(),
            cause: Some(format!(
                "seed {seed}: twin ordered public observation traces diverged: first={first:?}; second={second:?}"
            )),
        },
        (Err(first), Err(second)) => InvariantResult {
            name: name.to_owned(),
            status: InvariantStatus::Fail,
            tick: 0,
            host: HOST.to_owned(),
            cause: Some(format!(
                "seed {seed}: both owner runs failed: first={first}; second={second}"
            )),
        },
        (Err(cause), Ok(trace)) | (Ok(trace), Err(cause)) => InvariantResult {
            name: name.to_owned(),
            status: InvariantStatus::Fail,
            tick: u64::try_from(trace.len()).unwrap_or(u64::MAX),
            host: HOST.to_owned(),
            cause: Some(format!(
                "seed {seed}: one owner run failed: {cause}; peer_trace={trace:?}"
            )),
        },
    }
}
