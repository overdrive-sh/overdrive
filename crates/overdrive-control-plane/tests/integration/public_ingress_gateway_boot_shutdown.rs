//! Public-ingress gateway — boot and shutdown through the ordinary production
//! composition: S-PIG-59 and the disabled arm of S-PIG-100 (over the host
//! bundle, `run_server`), and the enabled-boot arms of S-PIG-02 (I), 54, 57
//! (I), 58, 60 (I), 61, 62, 74 (b), (c) and 100 (over the supplied-owner
//! `run_server_with_obs_and_driver`, gateway enabled through
//! `ServerConfig.gateway`).
//!
//! Every body enters through the production entry point and observes the real
//! operator HTTPS surface, the tracing stage events and the Sim relay and
//! connect-dataplane transcripts. The activation gate is gateway-local (R-65):
//! the S-PIG-58 bodies count the connect-dataplane `open_upstream` and
//! `take_receipt` calls and the client `authenticate` calls and require zero
//! of each before the first public request. Faults enter only through existing driven
//! ports: the Sim relay's substrate lie, the Sim connect dataplane's typed
//! `fail_next`, a refusing Sim driver, and a delegating CA that
//! refuses SVID issuance. Nothing is installed that production does not
//! install: the Service and the Route enter through `POST /v1/workloads` and
//! `POST /v1/routes`, and the public client is a real loopback TCP connection
//! to the gateway's own address.
//!
//! | Body | Scenario | State |
//! |---|---|---|
//! | `a_disabled_gateway_boots_and_shuts_down_exactly_as_before` | S-PIG-59 (boundary 6) | expected GREEN, no `#[ignore]` |
//! | `a_disabled_gateway_never_probes_the_relay` | S-PIG-100, disabled arm | ignored until `SimGatewayRelay` lands |
//! | `a_service_with_no_backend_is_answered_503_twice_with_an_empty_second_delta` | S-PIG-02 (I), 54 | ignored |
//! | `a_failing_sentinel_probe_keeps_the_public_port_unbound_and_serve_unaffected` | S-PIG-57 (I), S-PIG-74 (a) | ignored |
//! | `a_frontend_with_no_backend_is_bind_valid_and_the_listener_serves_503` | S-PIG-58 (a) | ignored |
//! | `a_selected_backend_that_refuses_mtls_is_bind_valid_and_its_first_request_is_one_502` | S-PIG-58 (b), R-65 | ignored |
//! | `a_route_replacement_behind_a_backend_that_refuses_mtls_is_promoted_and_drains_the_prior_generation` | S-PIG-58 (c), R-65 | ignored |
//! | `enabled_boot_probes_the_relay_before_the_connect_path_and_the_gateway_stage_runs_first` | S-PIG-60 (I) | ignored |
//! | `a_gated_gateway_never_requests_serve_shutdown` | S-PIG-61 | ignored |
//! | `the_workload_surface_advances_identically_disabled_enabled_and_gated` | S-PIG-62 | ignored |
//! | `a_gateway_identity_that_never_becomes_current_keeps_the_public_port_unbound` | S-PIG-74 (b) | ignored |
//! | `a_probe_that_times_out_keeps_the_public_port_unbound` | S-PIG-74 (c) | ignored |
//! | `a_refused_relay_probe_keeps_the_public_port_unbound_and_the_connect_probe_unrun` | S-PIG-100 | ignored |
//! | `the_relay_probe_runs_once_per_boot_across_a_route_replacement` | S-PIG-100 | ignored |
//! | `an_undecodable_stored_route_is_a_typed_500_route_decode_on_post_and_delete` | S-PIG-140 (R-67), garbage arm | `#[ignore]` pending `route-set-error-mapping` |
//! | `a_well_formed_envelope_holding_an_invalid_route_is_a_typed_500_route_decode` | S-PIG-140 (R-67), invalid-Route arm | `#[ignore]` pending `route-set-error-mapping` and a pinned read-side Route validation |
//! | `a_route_owner_that_ended_in_shutdown_answers_503_gateway_owner_unavailable_on_both_methods` | S-PIG-141 (R-67), shutdown arm | `#[ignore]` pending `route-set-error-mapping` |
//!
//! # Bind-address table (R-6)
//!
//! A composition that reaches promotion binds its own `127.0.0.0/8` address on
//! TCP/443. This file owns second octets 97 and 152..=168; the gateway
//! integration files own 41..=62, 68..=96 and 130..=132, the in-crate drivers
//! 63..=67 and 98, the sim acceptance file 120 and 121, the conformance binary
//! 100..=119.
//!
//! | `T` | test |
//! |---|---|
//! | 97 | disabled boot: the configured-but-unused address stays unbound (S-PIG-59) |
//! | 152 | S-PIG-02 / 54 |
//! | 153 | S-PIG-57 |
//! | 154 | S-PIG-58 (a) |
//! | 167 | S-PIG-58 (b) |
//! | 168 | S-PIG-58 (c) |
//! | 155 | S-PIG-60 |
//! | 156 | S-PIG-61 (gated) |
//! | 158, 159, 160 | S-PIG-62 (disabled, enabled, gated) |
//! | 163, 164 | S-PIG-74 (b), (c); (a), a hook that does not fire (`GatewayConnectDataplane::probe` returns `Unavailable`, sentinel receipt absent), is the S-PIG-57 (I) stimulus. 161 is unused |
//! | 165, 166 | S-PIG-100 (refused relay probe; once per boot) |
//! | 169, 170, 171 | S-PIG-140 (garbage; invalid Route), S-PIG-141 (shutdown). No Service is deployed, so none binds TCP/443 |

#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_const_for_fn,
    reason = "CONTRACT_SHAPE metadata; fixture preconditions are asserted"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream};
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_control_plane::api::{
    AllocStateWire, AllocStatusResponse, ClusterStatus, GatewayApplicationUnavailableCauseBody,
    GatewayConnectPathStatusBody, GatewayIdentityStatusBody, GatewayListenerStatusBody,
    GatewayStatusResponse, SubmitRouteResponse, SubmitWorkloadRequest,
};
use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::{
    BootAdapters, ComposedDataplane, DataplaneComposer, DataplanePlan, GatewayAdapters,
    ServerHandle, run_server_with_obs_and_driver,
};
use overdrive_core::aggregate::{DriverInput, ResourcesInput, VmInput};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::ca::kek::KEK_LEN;
use overdrive_core::ca::root_key_envelope::KekId;
use overdrive_core::guest_network::GuestNetworkExecWiring;
use overdrive_core::id::{BackendId, NodeId, PublicCertifiedKeyId, SpiffeId};
use overdrive_core::public_ingress::{
    PublicHostname, PublicRouteInput, ServiceKey, ServiceListenerReferenceInput, SocketCookie,
};
use overdrive_core::public_ingress::{PublicRouteSetV1, Route};
use overdrive_core::reconcilers::Reconciler;
use overdrive_core::traits::ca::{
    Ca, CaError, CaFactory, IntermediateHandle, RootCaHandle, SvidMaterial, SvidRequest,
    TrustBundle,
};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_dataplane::allocators::VipRange;
use overdrive_gateway::ports::{
    GatewayClientMtls, GatewayClientMtlsDeps, GatewayClientMtlsError, GatewayClientMtlsFactory,
    GatewayConnectCleanup, GatewayConnectCleanupSweep, GatewayConnectDataplane,
    GatewayConnectDataplaneError, GatewayConnectIntent, GatewayRelay, GatewaySelectionReceipt,
    GatewayUpstream, PreparedUpstream,
};
use overdrive_gateway::{GatewayConfig, ManualCertifiedKeyConfig};
use overdrive_host::ca::PublicCertifiedKeyAeadCodec;
use overdrive_reconcilers::GatewaySvidLifecycle;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::gateway::{
    ConnectOperation, LegRole, LegScript, PublicLegScript, SimGatewayAdapters,
    SimGatewayConnectDataplane, SimGatewayRelay, SubstrateLie,
};
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::kek::SimKek;
use overdrive_store_local::LocalIntentStore;
use serial_test::serial;
use tempfile::TempDir;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

// ---------------------------------------------------------------------------
// Event capture
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct EventRow {
    name: String,
    fields: BTreeMap<String, String>,
}

#[derive(Default)]
struct FieldVisitor(BTreeMap<String, String>);

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}").trim_matches('"').to_owned());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

#[derive(Clone, Default)]
struct EventCollector(Arc<Mutex<Vec<EventRow>>>);

impl<S: Subscriber> Layer<S> for EventCollector {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let name =
            visitor.0.get("name").cloned().unwrap_or_else(|| event.metadata().name().to_owned());
        self.0.lock().expect("event collector lock").push(EventRow { name, fields: visitor.0 });
    }
}

impl EventCollector {
    fn refusal_reasons(&self) -> Vec<String> {
        self.0
            .lock()
            .expect("event collector lock")
            .iter()
            .filter(|row| row.name == "health.startup.refused")
            .filter_map(|row| row.fields.get("reason").cloned())
            .collect()
    }

    /// The `stage` field of every shutdown stage event, in emission order.
    fn shutdown_stages(&self) -> Vec<String> {
        self.0
            .lock()
            .expect("event collector lock")
            .iter()
            .filter(|row| {
                row.name == "serve.shutdown.stage_begin"
                    || row.name == "serve.shutdown.stage_complete"
            })
            .filter_map(|row| row.fields.get("stage").cloned())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Operator HTTPS client
// ---------------------------------------------------------------------------

fn client_trusting(ca_pem: &str) -> reqwest::Client {
    let cert = reqwest::Certificate::from_pem(ca_pem.as_bytes()).expect("parse CA certificate PEM");
    reqwest::Client::builder()
        .add_root_certificate(cert)
        .https_only(true)
        .use_rustls_tls()
        .build()
        .expect("build reqwest client")
}

/// The CA PEM `run_server` wrote to the ADR-0019 trust triple.
fn read_ca_from_trust_triple(operator_config_dir: &std::path::Path) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    let config_path = operator_config_dir.join(".overdrive").join("config");
    let text = std::fs::read_to_string(&config_path).expect("read the trust triple");
    let doc: toml::Value = toml::from_str(&text).expect("parse trust triple TOML");
    let ca_b64 = doc
        .get("contexts")
        .and_then(toml::Value::as_array)
        .and_then(|contexts| {
            contexts.iter().find(|c| c.get("name").and_then(toml::Value::as_str) == Some("local"))
        })
        .and_then(|context| context.get("ca"))
        .and_then(toml::Value::as_str)
        .expect("a local context with a ca field");
    String::from_utf8(BASE64.decode(ca_b64).expect("base64 ca")).expect("ca PEM is UTF-8")
}

// ---------------------------------------------------------------------------
// The node under test
// ---------------------------------------------------------------------------

struct Node {
    tmp: TempDir,
}

impl Node {
    fn new() -> Self {
        Self { tmp: TempDir::new().expect("tempdir") }
    }

    /// The disabled configuration: `ServerConfig::new()`'s default gateway.
    fn config(&self) -> overdrive_control_plane::ServerConfig {
        let data_dir = self.tmp.path().join("data");
        let operator_config_dir = self.tmp.path().join("conf");
        std::fs::create_dir_all(&data_dir).expect("create data dir");
        std::fs::create_dir_all(&operator_config_dir).expect("create config dir");
        super::node_adapters::test_config(data_dir, operator_config_dir)
    }

    fn operator_config_dir(&self) -> std::path::PathBuf {
        self.tmp.path().join("conf")
    }
}

/// The addresses this process is listening on at TCP port 443: the listening
/// sockets in the network namespace (`/proc/net/tcp`, state `0A`) whose inode is
/// one of this process's own descriptors, so a concurrent test process in the
/// same namespace is never counted.
fn own_listeners_on_443() -> std::collections::BTreeSet<String> {
    let own_inodes: std::collections::BTreeSet<String> = std::fs::read_dir("/proc/self/fd")
        .expect("list this process's descriptors")
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_link(entry.path()).ok())
        .filter_map(|target| {
            target
                .to_str()
                .and_then(|text| text.strip_prefix("socket:["))
                .and_then(|text| text.strip_suffix(']'))
                .map(str::to_owned)
        })
        .collect();
    let table = std::fs::read_to_string("/proc/net/tcp").expect("read the TCP socket table");
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let local = *fields.get(1)?;
            let state = *fields.get(3)?;
            let inode = *fields.get(9)?;
            (state == "0A" && local.ends_with(":01BB") && own_inodes.contains(inode))
                .then(|| local.to_owned())
        })
        .collect()
}

// ---------------------------------------------------------------------------
// S-PIG-59
// ---------------------------------------------------------------------------

/// S-PIG-59 (boundary 6) — With `ServerConfig.gateway == None` the boot is the
/// pre-feature boot: no gateway reconciler is registered, no refusal is
/// emitted, no public socket is bound, and the graceful shutdown runs no
/// `gateway` stage.
///
/// Expected-GREEN preservation row: it holds on the code as scaffolded because
/// the disabled composition constructs no gateway owner, and it keeps holding
/// when the gateway lands.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[serial(cgroup)]
async fn a_disabled_gateway_boots_and_shuts_down_exactly_as_before() {
    let node = Node::new();
    let config = node.config();
    assert!(config.gateway.is_none(), "the fixture's configuration leaves the gateway disabled");
    let collector = EventCollector::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(collector.clone()));

    let listeners_before = own_listeners_on_443();
    let adapters = super::node_adapters::node_adapters(&config);
    let handle = overdrive_control_plane::run_server(config, adapters)
        .await
        .expect("a disabled gateway boots as before");

    // The operator listener serves, and the registry holds no gateway reconciler.
    let bound = handle.local_addr().await.expect("bound operator address");
    let client = client_trusting(&read_ca_from_trust_triple(&node.operator_config_dir()));
    let status: ClusterStatus = client
        .get(format!("https://localhost:{}/v1/cluster/info", bound.port()))
        .send()
        .await
        .expect("GET /v1/cluster/info")
        .json()
        .await
        .expect("ClusterStatus body");
    assert!(
        !status.reconcilers.iter().any(|name| name == <GatewaySvidLifecycle as Reconciler>::NAME),
        "no gateway reconciler is registered with the gateway disabled: {:?}",
        status.reconcilers
    );
    assert!(collector.refusal_reasons().is_empty(), "a disabled gateway emits no refusal");
    assert_eq!(
        own_listeners_on_443(),
        listeners_before,
        "no public TCP/443 socket is bound with the gateway disabled"
    );

    handle.shutdown(Duration::from_secs(1)).await.expect("graceful shutdown");
    assert!(
        !collector.shutdown_stages().iter().any(|stage| stage == "gateway"),
        "no gateway shutdown stage runs: {:?}",
        collector.shutdown_stages()
    );
}

// ---------------------------------------------------------------------------
// S-PIG-100, disabled arm
// ---------------------------------------------------------------------------

/// S-PIG-100 (disabled arm) — With the gateway disabled the relay is composed
/// into the node (`BootAdapters.gateway` is mandatory) but never called: the
/// Sim relay records zero probe calls across the whole boot and shutdown.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[serial(cgroup)]
async fn a_disabled_gateway_never_probes_the_relay() {
    let node = Node::new();
    let config = node.config();
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let relay = Arc::new(SimGatewayRelay::new(clock));
    let mut adapters = super::node_adapters::node_adapters(&config);
    adapters.boot.gateway.relay = Arc::clone(&relay) as Arc<dyn GatewayRelay>;

    let handle = overdrive_control_plane::run_server(config, adapters)
        .await
        .expect("a disabled gateway boots as before");
    assert_eq!(relay.probe_calls(), 0, "boot with the gateway disabled runs no relay probe");

    handle.shutdown(Duration::from_secs(1)).await.expect("graceful shutdown");
    assert_eq!(relay.probe_calls(), 0, "shutdown with the gateway disabled runs no relay probe");
}

// ===========================================================================
// The enabled composition
// ===========================================================================

const HOSTNAME: &str = "api.example.com";
const SERVICE_ID: &str = "api";
const SERVICE_PORT: u16 = 8080;
const ROUTE_ID: &str = "public-api";
const CERTIFIED_KEY_ID: &str = "api-origin";
/// The convergence cadence on the injected `SimClock`.
const TICK: Duration = Duration::from_millis(100);
/// Real-time ceiling for any wait on a spawned task.
const WAIT: Duration = Duration::from_secs(30);
/// The public request the scripted client sends.
const REQUEST: &[u8] = b"GET / HTTP/1.1\r\nHost: api.example.com\r\n\r\n";

/// The process-wide event collector. Each nextest test is its own process, so
/// the one global subscriber sees exactly this test's boot; spawned tasks on
/// other threads are captured, which a thread-local default would miss.
fn events() -> &'static EventCollector {
    static EVENTS: std::sync::OnceLock<EventCollector> = std::sync::OnceLock::new();
    EVENTS.get_or_init(|| {
        let collector = EventCollector::default();
        tracing::subscriber::set_global_default(
            tracing_subscriber::registry().with(collector.clone()),
        )
        .expect("install the process-wide event collector once");
        collector
    })
}

/// A test-local `DataplaneComposer` over the Sim dataplane that hands the boot
/// the connect capability the test chose (the Sim directly, or a spy over it).
struct FixtureComposer {
    dataplane: Arc<SimDataplane>,
    connect: Arc<dyn GatewayConnectDataplane>,
}

#[async_trait]
impl DataplaneComposer for FixtureComposer {
    async fn compose(
        &self,
        _plan: DataplanePlan<'_>,
    ) -> Result<ComposedDataplane, ControlPlaneError> {
        Ok(ComposedDataplane {
            dataplane: Arc::clone(&self.dataplane) as _,
            host_ipv4: Ipv4Addr::LOCALHOST,
            gateway_connect: Arc::clone(&self.connect),
        })
    }
}

/// Delegates to the Sim connect dataplane and records, at the start of every
/// connect probe, how many relay probes had already run (the G3 order), and how
/// many upstream sockets were opened and receipts taken through the port. The
/// Sim's own `probe` reaches its state directly, so these two counters see only
/// request-path calls: the gate makes none (R-65).
struct ProbeOrderSpy {
    inner: Arc<SimGatewayConnectDataplane>,
    relay: Arc<SimGatewayRelay>,
    relay_probes_seen_by_connect_probe: Arc<Mutex<Vec<u32>>>,
    /// `open_upstream` calls through the port.
    opens: Arc<AtomicU32>,
    /// `take_receipt` calls through the port.
    receipt_takes: Arc<AtomicU32>,
}

impl GatewayConnectCleanup for ProbeOrderSpy {
    fn cleanup(&self, intent: &GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError> {
        self.inner.cleanup(intent)
    }

    fn cleanup_all_gateway_intents(
        &self,
    ) -> Result<GatewayConnectCleanupSweep, GatewayConnectDataplaneError> {
        self.inner.cleanup_all_gateway_intents()
    }

    fn live_intent_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
        self.inner.live_intent_count()
    }

    fn live_receipts(
        &self,
    ) -> Result<BTreeMap<SocketCookie, GatewaySelectionReceipt>, GatewayConnectDataplaneError> {
        self.inner.live_receipts()
    }
}

#[async_trait]
impl GatewayConnectDataplane for ProbeOrderSpy {
    async fn probe(
        &self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Result<(), GatewayConnectDataplaneError> {
        self.relay_probes_seen_by_connect_probe
            .lock()
            .expect("probe log lock")
            .push(self.relay.probe_calls());
        self.inner.probe(deadline, clock).await
    }

    fn open_upstream(
        &self,
        key: ServiceKey,
    ) -> Result<PreparedUpstream, GatewayConnectDataplaneError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        self.inner.open_upstream(key)
    }

    fn take_receipt(
        &self,
        intent: &GatewayConnectIntent,
    ) -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError> {
        self.receipt_takes.fetch_add(1, Ordering::SeqCst);
        self.inner.take_receipt(intent)
    }

    fn selected_backend_identity(
        &self,
        backend_id: BackendId,
    ) -> Result<SpiffeId, GatewayConnectDataplaneError> {
        self.inner.selected_backend_identity(backend_id)
    }
}

/// A CA that mints the same roots as the Sim CA but refuses every workload
/// SVID: the one place the gateway identity can be kept from becoming Current
/// through an existing driven port. The nodes under test deploy no workload
/// that needs an SVID in that case.
struct RefusingSvidCa {
    inner: Arc<dyn Ca>,
}

impl Ca for RefusingSvidCa {
    fn root(&self) -> Result<RootCaHandle, CaError> {
        self.inner.root()
    }

    fn issue_intermediate(&self, node: &NodeId) -> Result<IntermediateHandle, CaError> {
        self.inner.issue_intermediate(node)
    }

    fn issue_svid(&self, _req: &SvidRequest) -> Result<SvidMaterial, CaError> {
        Err(CaError::SigningFailed { reason: "svid-issuance-fault-marker".to_owned() })
    }

    fn trust_bundle(&self) -> Result<TrustBundle, CaError> {
        self.inner.trust_bundle()
    }
}

struct RefusingSvidCaFactory;

impl CaFactory for RefusingSvidCaFactory {
    fn build(&self, entropy: Arc<dyn Entropy>) -> Arc<dyn Ca> {
        Arc::new(RefusingSvidCa { inner: overdrive_sim::adapters::SimCaFactory.build(entropy) })
    }
}

/// Delegates to the Sim exact-peer client and counts every `authenticate`. The
/// unscripted Sim client presents no peer, so it refuses every selected
/// backend (`PeerCertificateMissing`): the backend whose mTLS fails.
struct CountingMtlsFactory {
    inner: Arc<dyn GatewayClientMtlsFactory>,
    calls: Arc<AtomicU32>,
}

impl GatewayClientMtlsFactory for CountingMtlsFactory {
    fn client_mtls(&self, deps: GatewayClientMtlsDeps) -> Arc<dyn GatewayClientMtls> {
        Arc::new(CountingMtls {
            inner: self.inner.client_mtls(deps),
            calls: Arc::clone(&self.calls),
        })
    }
}

struct CountingMtls {
    inner: Arc<dyn GatewayClientMtls>,
    calls: Arc<AtomicU32>,
}

#[async_trait]
impl GatewayClientMtls for CountingMtls {
    async fn authenticate(
        &self,
        connected: OwnedFd,
        expected_peer: SpiffeId,
        deadline: Instant,
    ) -> Result<GatewayUpstream, GatewayClientMtlsError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.authenticate(connected, expected_peer, deadline).await
    }
}

/// What a world is built with. Every field is a choice at an existing driven
/// port or in `ServerConfig`; the defaults are the healthy enabled node.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent fixture switches, one per existing driven port"
)]
struct Spec {
    octet: u8,
    enabled: bool,
    relay_lie: Option<SubstrateLie>,
    probe_fault: Option<GatewayConnectDataplaneError>,
    refuse_svid: bool,
    spy: bool,
    /// The Sim driver refuses to start any allocation, so the Service never
    /// reaches Running and no backend is ever programmed, however far the
    /// injected clock advances.
    driver_refuses_start: bool,
    /// The Service VIP range is `127.<octet>.2.0/24`, so the Service frontend
    /// is a loopback address the connector's real `connect(2)` reaches
    /// directly. The Sim dataplane does not rewrite the destination, so a
    /// default `10.96.x.y` VIP would leave the dial hanging or failing by
    /// whatever route the host has. With nothing listening on the frontend the
    /// kernel refuses at once; a test that wants the dial to succeed binds the
    /// frontend itself ([`FrontendDrain`]).
    frontend_on_loopback: bool,
}

impl Spec {
    /// The Service VIP range this composition allocates from.
    fn vip_range(&self) -> VipRange {
        if !self.frontend_on_loopback {
            return VipRange::default();
        }
        VipRange::new(
            vec![format!("127.{}.2.0/24", self.octet).parse().expect("loopback VIP range")],
            BTreeSet::new(),
        )
        .expect("a loopback VIP range with no reserved address")
    }

    fn enabled(octet: u8) -> Self {
        Self {
            octet,
            enabled: true,
            relay_lie: None,
            probe_fault: None,
            refuse_svid: false,
            spy: false,
            driver_refuses_start: false,
            frontend_on_loopback: false,
        }
    }
}

/// One production boot of the node, plus the doubles it was composed over.
struct World {
    _tmp: TempDir,
    octet: u8,
    clock: Arc<SimClock>,
    dataplane: Arc<SimDataplane>,
    connect: Arc<SimGatewayConnectDataplane>,
    sim: SimGatewayAdapters,
    relay_probes_seen_by_connect_probe: Arc<Mutex<Vec<u32>>>,
    /// `open_upstream` calls, counted when the spy is installed.
    opens: Arc<AtomicU32>,
    /// `take_receipt` calls, counted when the spy is installed.
    receipt_takes: Arc<AtomicU32>,
    /// `GatewayClientMtls::authenticate` calls.
    authenticate_calls: Arc<AtomicU32>,
    handle: Option<ServerHandle>,
    /// The node's intent store, the same instance the boot composes: the
    /// driven port a Route-set fault is delivered through (S-PIG-140).
    intent: Arc<LocalIntentStore>,
    client: reqwest::Client,
    base: String,
}

impl World {
    #[allow(
        clippy::too_many_lines,
        reason = "one production boot over every double the cases share"
    )]
    async fn boot(spec: Spec) -> Self {
        // Install the process-wide collector before the boot emits its first event.
        events();
        let tmp = TempDir::new().expect("tempdir");
        let data_dir = tmp.path().join("data");
        let operator_config_dir = tmp.path().join("conf");
        std::fs::create_dir_all(&data_dir).expect("create data dir");
        std::fs::create_dir_all(&operator_config_dir).expect("create config dir");
        let (chain_path, key_path) = write_public_key_files(tmp.path());
        let gateway = spec.enabled.then(|| {
            GatewayConfig::new(
                Ipv4Addr::new(127, spec.octet, 0, 1),
                ManualCertifiedKeyConfig::new(
                    PublicCertifiedKeyId::new(CERTIFIED_KEY_ID).expect("key id"),
                    chain_path,
                    key_path,
                ),
            )
        });
        let config = overdrive_control_plane::ServerConfig {
            tick_cadence: TICK,
            gateway,
            vip_range: spec.vip_range(),
            ..super::node_adapters::test_config(data_dir, operator_config_dir.clone())
        };

        let clock = Arc::new(SimClock::new());
        let clock_port: Arc<dyn Clock> = clock.clone();
        let dataplane = Arc::new(SimDataplane::new());
        let connect = Arc::new(SimGatewayConnectDataplane::new(Arc::clone(&dataplane)));
        let sim = SimGatewayAdapters::new(Arc::clone(&clock_port));
        sim.sim_relay().present_substrate(spec.relay_lie);
        if let Some(fault) = spec.probe_fault {
            connect.fail_next(ConnectOperation::Probe, fault);
        }
        let probe_log = Arc::new(Mutex::new(Vec::new()));
        let opens = Arc::new(AtomicU32::new(0));
        let receipt_takes = Arc::new(AtomicU32::new(0));
        let authenticate_calls = Arc::new(AtomicU32::new(0));
        let capability: Arc<dyn GatewayConnectDataplane> = if spec.spy {
            Arc::new(ProbeOrderSpy {
                inner: Arc::clone(&connect),
                relay: Arc::clone(sim.sim_relay()),
                relay_probes_seen_by_connect_probe: Arc::clone(&probe_log),
                opens: Arc::clone(&opens),
                receipt_takes: Arc::clone(&receipt_takes),
            })
        } else {
            Arc::clone(&connect) as _
        };

        let mut boot: BootAdapters = super::node_adapters::boot_adapters(&config);
        let intent = Arc::clone(boot.stores.intent.store());
        boot.clock = Arc::clone(&clock_port);
        boot.dataplane =
            Arc::new(FixtureComposer { dataplane: Arc::clone(&dataplane), connect: capability });
        boot.gateway = GatewayAdapters {
            certified_key_codec: Arc::new(PublicCertifiedKeyAeadCodec::new(Arc::new(
                SimKek::new().with(
                    &KekId::new("overdrive-public-certified-key").expect("custody KEK id"),
                    [0x5a; KEK_LEN],
                ),
            ))),
            client_mtls: Arc::new(CountingMtlsFactory {
                inner: sim.client_mtls(),
                calls: Arc::clone(&authenticate_calls),
            }),
            relay: sim.relay(),
        };
        if spec.refuse_svid {
            boot.ca.factory = Arc::new(RefusingSvidCaFactory);
        }
        let sim_driver = SimDriver::with_clock(DriverType::Vm, Arc::clone(&clock_port));
        let driver: Arc<dyn Driver> = Arc::new(if spec.driver_refuses_start {
            sim_driver.fail_on_start_with("the Service has no startable allocation".to_owned())
        } else {
            sim_driver
        });
        let handle = run_server_with_obs_and_driver(
            config,
            boot,
            driver,
            Arc::new(SimSharedGuestNetworkOwner::default()),
            GuestNetworkExecWiring::new(Arc::clone(&clock_port)),
        )
        .await
        .expect("the enabled node boots; a gated gateway never refuses the boot");
        let bound = handle.local_addr().await.expect("bound operator address");
        let client = client_trusting(&read_ca_from_trust_triple(&operator_config_dir));
        Self {
            _tmp: tmp,
            octet: spec.octet,
            clock,
            dataplane,
            connect,
            sim,
            relay_probes_seen_by_connect_probe: probe_log,
            opens,
            receipt_takes,
            authenticate_calls,
            handle: Some(handle),
            intent,
            client,
            base: format!("https://localhost:{}", bound.port()),
        }
    }

    fn bind(&self) -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, self.octet, 0, 1), 443))
    }

    /// Whether this process listens on the gateway's address.
    fn listening(&self) -> bool {
        own_listeners_on_443().contains(&format!("{:02X}{:02X}{:02X}7F:01BB", 1, 0, self.octet))
    }

    /// Whether a connection to the gateway's address is refused.
    fn refused(&self) -> bool {
        TcpStream::connect_timeout(&self.bind(), Duration::from_millis(500))
            .is_err_and(|error| error.kind() == std::io::ErrorKind::ConnectionRefused)
    }

    /// Advance the injected clock one tick and let the spawned tasks run.
    async fn settle(&self, step: Duration) {
        self.clock.tick(step);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    async fn operator_get<T: serde::de::DeserializeOwned>(&self, path: &str) -> T {
        self.client
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap_or_else(|error| panic!("GET {path}: {error}"))
            .json()
            .await
            .unwrap_or_else(|error| panic!("decode GET {path}: {error}"))
    }

    async fn status(&self) -> GatewayStatusResponse {
        self.operator_get("/v1/gateway/status").await
    }

    /// Poll the gateway status, advancing the injected clock by `step` per poll,
    /// until `done` holds.
    async fn wait_status(
        &self,
        what: &str,
        step: Duration,
        done: impl Fn(&GatewayStatusResponse) -> bool,
    ) -> GatewayStatusResponse {
        let deadline = Instant::now() + WAIT;
        loop {
            let status = self.status().await;
            if done(&status) {
                return status;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}: {status:?}");
            self.settle(step).await;
        }
    }

    async fn deploy_service(&self) {
        let spec = SubmitSpecInput::Service(ServiceSpecInput {
            id: SERVICE_ID.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 134_217_728 },
            driver: DriverInput::Vm(VmInput {
                command: "/bin/true".to_owned(),
                args: Vec::new(),
                kernel: "/kernel".to_owned(),
                rootfs: "/rootfs".to_owned(),
            }),
            listeners: vec![ListenerInput { port: SERVICE_PORT, protocol: "tcp".to_owned() }],
            startup_probes: Vec::new(),
            readiness_probes: Vec::new(),
            liveness_probes: Vec::new(),
        });
        let response = self
            .client
            .post(format!("{}/v1/workloads", self.base))
            .json(&SubmitWorkloadRequest { spec })
            .send()
            .await
            .expect("POST /v1/workloads");
        assert!(response.status().is_success(), "the Service is admitted: {response:?}");
    }

    async fn declare_route(&self, path: &str) -> SubmitRouteResponse {
        let input = PublicRouteInput {
            id: ROUTE_ID.to_owned(),
            hostname: HOSTNAME.to_owned(),
            path: path.to_owned(),
            path_match: "segment_prefix".to_owned(),
            certified_key: CERTIFIED_KEY_ID.to_owned(),
            target: ServiceListenerReferenceInput {
                service: SERVICE_ID.to_owned(),
                port: SERVICE_PORT,
                protocol: "tcp".to_owned(),
            },
        };
        let response = self
            .client
            .post(format!("{}/v1/routes", self.base))
            .json(&input)
            .send()
            .await
            .expect("POST /v1/routes");
        assert!(response.status().is_success(), "the Route is accepted: {response:?}");
        response.json().await.expect("SubmitRouteResponse body")
    }

    /// The healthy enabled node with the Service deployed and the Route
    /// declared, waited until the listener is bound.
    async fn promoted(&self) -> GatewayStatusResponse {
        self.deploy_service().await;
        self.declare_route("/").await;
        self.wait_status("the listener to bind", TICK, |status| {
            status.listener_bound
                && status.application.as_ref().is_some_and(|row| {
                    matches!(row.listener, GatewayListenerStatusBody::Bound { .. })
                })
        })
        .await
    }

    /// One public request over a real loopback connection to the gateway; the
    /// bytes the gateway wrote to the public leg are the response.
    async fn public_response(&self) -> Vec<u8> {
        let relay = self.sim.sim_relay();
        let first = relay.leg_transcripts().len() as u64;
        relay.script_public_connection(PublicLegScript {
            sni: PublicHostname::new(HOSTNAME).expect("SNI hostname"),
            inbound: LegScript::new().octets(REQUEST).close_notify(),
        });
        let _client = TcpStream::connect(self.bind()).expect("connect to the gateway listener");
        let deadline = Instant::now() + WAIT;
        loop {
            let public = relay
                .leg_transcripts()
                .into_iter()
                .find(|leg| leg.id.0 >= first && leg.role == LegRole::Public && leg.closed);
            if let Some(leg) = public {
                return leg.written;
            }
            assert!(Instant::now() < deadline, "timed out waiting for the public exchange to end");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// The Progress of the workload surface: allocation state, hydration of the
    /// Service's backends into the dataplane, whether the allocation carries a
    /// Stable claim, and whether the operator listener answers.
    async fn progress(&self) -> Progress {
        let listing: AllocStatusResponse =
            self.operator_get(&format!("/v1/allocs?job={SERVICE_ID}")).await;
        let row = listing.rows.first();
        let hydrated_backends: usize = self
            .dataplane
            .service_vip_keys()
            .into_iter()
            .map(|vip| self.dataplane.service_backends(vip).map_or(0, |backends| backends.len()))
            .sum();
        let cluster: ClusterStatus = self.operator_get("/v1/cluster/info").await;
        Progress {
            allocation_running: row.is_some_and(|row| row.state == AllocStateWire::Running),
            allocation_stable: row.is_some_and(|row| {
                matches!(
                    row.terminal,
                    Some(overdrive_core::transition_reason::TerminalCondition::Stable { .. })
                )
            }),
            backends_hydrated: hydrated_backends,
            operator_listener_answers: !cluster.reconcilers.is_empty(),
        }
    }

    /// The premise of a no-backend case, observed: no allocation is Running
    /// and no backend is programmed into the dataplane.
    async fn assert_no_backend_programmed(&self) {
        let observed = self.progress().await;
        assert!(
            !observed.allocation_running && observed.backends_hydrated == 0,
            "the Service has no programmed backend: {observed:?}"
        );
    }

    /// Advance until the Service is Running and its backend is programmed into
    /// the dataplane, observed through the allocation listing and the Sim
    /// dataplane.
    async fn wait_for_programmed_backend(&self) -> Progress {
        let deadline = Instant::now() + WAIT;
        loop {
            let observed = self.progress().await;
            if observed.allocation_running && observed.backends_hydrated > 0 {
                return observed;
            }
            assert!(Instant::now() < deadline, "timed out waiting for a programmed backend");
            self.settle(TICK).await;
        }
    }

    /// `open_upstream` calls so far (the spy must be installed).
    fn opens(&self) -> u32 {
        self.opens.load(Ordering::SeqCst)
    }

    /// `take_receipt` calls so far (the spy must be installed).
    fn receipt_takes(&self) -> u32 {
        self.receipt_takes.load(Ordering::SeqCst)
    }

    /// `GatewayClientMtls::authenticate` calls so far.
    fn authenticate_calls(&self) -> u32 {
        self.authenticate_calls.load(Ordering::SeqCst)
    }

    /// The activation gate is gateway-local (R-65): before any public request
    /// it has made no `open_upstream`, no `take_receipt` and no `authenticate`
    /// call (the spy must be installed).
    fn assert_gate_made_no_service_connection(&self) {
        assert_eq!(
            (self.opens(), self.receipt_takes(), self.authenticate_calls()),
            (0, 0, 0),
            "the gate made (open_upstream, take_receipt, authenticate) calls"
        );
    }

    /// Graceful shutdown on the injected clock.
    async fn shutdown(&mut self) {
        let handle = self.handle.take().expect("the server handle");
        let clock = Arc::clone(&self.clock);
        let mut shutdown = std::pin::pin!(handle.shutdown(Duration::from_secs(5)));
        loop {
            tokio::select! {
                biased;
                result = &mut shutdown => {
                    result.expect("graceful shutdown");
                    return;
                }
                () = tokio::time::sleep(Duration::from_millis(5)) => clock.tick(TICK),
            }
        }
    }
}

/// Accepts every connection made to a Service frontend, so the connector's
/// real `connect(2)` completes and the connection stays open and silent until
/// the drain drops (as in the gateway relay integration file).
struct FrontendDrain {
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<Vec<TcpStream>>>,
}

impl FrontendDrain {
    /// Binds `address` (a loopback address of this composition's octet) and
    /// serves it until dropped.
    fn start(address: SocketAddr) -> Self {
        let listener = std::net::TcpListener::bind(address).expect("bind the Service frontend");
        listener.set_nonblocking(true).expect("nonblocking frontend listener");
        let stop = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&stop);
        let join = std::thread::Builder::new()
            .name("frontend-drain".to_owned())
            .spawn(move || {
                let mut held = Vec::new();
                while !observed.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => held.push(stream),
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(error) => panic!("Service frontend accept failed: {error}"),
                    }
                }
                held
            })
            .expect("spawn the frontend drain");
        Self { stop, join: Some(join) }
    }
}

impl Drop for FrontendDrain {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take()
            && let Err(panic) = join.join()
            && !std::thread::panicking()
        {
            std::panic::resume_unwind(panic);
        }
    }
}

/// Allocation, hydration and operator-surface progress of the Service.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Progress {
    allocation_running: bool,
    allocation_stable: bool,
    backends_hydrated: usize,
    operator_listener_answers: bool,
}

/// The operator-provided public key material the production serve flags name:
/// a leaf for [`HOSTNAME`], leaf-first chain, PKCS#8 key at mode 0600.
fn write_public_key_files(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt as _;
    let chain_path = dir.join("api-origin-chain.pem");
    let key_path = dir.join("api-origin-key.pem");
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("test key");
    let mut params =
        rcgen::CertificateParams::new(vec![HOSTNAME.to_owned()]).expect("certificate params");
    params.is_ca = rcgen::IsCa::NoCa;
    params.not_before = rcgen::date_time_ymd(2020, 1, 1);
    params.not_after = rcgen::date_time_ymd(2100, 1, 1);
    let cert = params.self_signed(&key).expect("test certificate");
    std::fs::write(&chain_path, cert.pem()).expect("write certificate");
    std::fs::write(&key_path, key.serialize_pem()).expect("write private key");
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
        .expect("private-key mode");
    (chain_path, key_path)
}

fn unavailable_is(
    status: &GatewayStatusResponse,
    expected: fn(&GatewayApplicationUnavailableCauseBody) -> bool,
) -> bool {
    status.application.as_ref().and_then(|row| row.unavailable.as_ref()).is_some_and(expected)
}

fn connect_path_unavailable(status: &GatewayStatusResponse) -> bool {
    unavailable_is(status, |cause| {
        matches!(cause, GatewayApplicationUnavailableCauseBody::ConnectPathUnavailable)
    })
}

/// The shared assertions of a keep-unbound case: TCP/443 unbound, serve and
/// workloads unaffected.
async fn assert_unbound_and_serve_unaffected(world: &World) {
    assert!(world.refused(), "TCP/443 stays unbound: a connection is refused");
    assert!(!world.listening(), "this process holds no listener on the gateway address");
    let cluster: ClusterStatus = world.operator_get("/v1/cluster/info").await;
    assert!(!cluster.reconcilers.is_empty(), "the operator listener serves");
    world.deploy_service().await;
}

// ---------------------------------------------------------------------------
// S-PIG-02 (I arm) and S-PIG-54
// ---------------------------------------------------------------------------

/// S-PIG-02 (I arm), S-PIG-54 — a Route whose Service has no programmed
/// backend is answered 503 with no fallback, and the same request a second
/// time has an empty delta: identical bytes, no live intent or receipt left,
/// the status row byte-equal, and the injected clock unmoved.
///
/// The Sim driver refuses to start the Service's allocation, so it never
/// reaches Running however far the clock advances and `update_service` is
/// never issued: the consuming-port outcome is `NoBackend`. The premise is
/// observed before each request through the allocation listing and the Sim
/// dataplane.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn a_service_with_no_backend_is_answered_503_twice_with_an_empty_second_delta() {
    let mut world = World::boot(Spec {
        driver_refuses_start: true,
        frontend_on_loopback: true,
        ..Spec::enabled(152)
    })
    .await;
    world.promoted().await;
    world.assert_no_backend_programmed().await;

    let clock_at_first_request = world.clock.now();
    let first = world.public_response().await;
    assert_eq!(
        world.clock.now(),
        clock_at_first_request,
        "the first request resolves on the refused loopback frontend and never waits"
    );
    assert!(
        first.starts_with(b"HTTP/1.1 503"),
        "a Service with no selectable backend is a 503, got {:?}",
        String::from_utf8_lossy(&first[..first.len().min(40)])
    );

    let status_before = serde_json::to_string(&world.status().await).expect("status JSON");
    let clock_before = world.clock.now();
    let live_before = (
        world.connect.live_intent_count().expect("live intents"),
        world.connect.live_receipts().expect("live receipts").len(),
        world.sim.sim_relay().live_leg_count(),
        world.sim.sim_relay().live_pipe_count(),
    );
    assert_eq!(live_before, (0, 0, 0, 0), "the first request leaves no residue");

    let second = world.public_response().await;

    assert_eq!(second, first, "the second 503 is byte-identical to the first");
    assert_eq!(
        serde_json::to_string(&world.status().await).expect("status JSON"),
        status_before,
        "the status row and Applied Generation are unchanged by the second request"
    );
    assert_eq!(world.clock.now(), clock_before, "no sleep or poll moved the injected clock");
    assert_eq!(
        (
            world.connect.live_intent_count().expect("live intents"),
            world.connect.live_receipts().expect("live receipts").len(),
            world.sim.sim_relay().live_leg_count(),
            world.sim.sim_relay().live_pipe_count(),
        ),
        live_before,
        "no intent, receipt, leg or pipe outlives the second request"
    );
    world.shutdown().await;
}

// ---------------------------------------------------------------------------
// S-PIG-57 (I arm)
// ---------------------------------------------------------------------------

/// S-PIG-57 (I arm), S-PIG-74 (a) — a sentinel probe that fails keeps TCP/443
/// unbound, status names the redacted cause, the startup-refused event carries
/// reason `gateway.connect_probe`, and serve and workloads are unaffected.
///
/// The fault is the Sim connect dataplane's typed `fail_next` at the probe
/// operation, the existing driven-port result: `GatewayConnectDataplane::probe`
/// returns `Unavailable` because the sentinel receipt is absent (the hook does
/// not fire). This is the S-PIG-74 (a) stimulus.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn a_failing_sentinel_probe_keeps_the_public_port_unbound_and_serve_unaffected() {
    let mut spec = Spec::enabled(153);
    spec.probe_fault = Some(GatewayConnectDataplaneError::Unavailable);
    let mut world = World::boot(spec).await;
    world.deploy_service().await;
    world.declare_route("/").await;

    let status = world.wait_status("the connect-path cause", TICK, connect_path_unavailable).await;

    assert!(!status.listener_bound, "the listener is unbound");
    assert!(
        matches!(
            status.application.as_ref().map(|row| &row.connect_path),
            Some(GatewayConnectPathStatusBody::Unavailable)
        ),
        "the connect path is reported Unavailable: {status:?}"
    );
    assert_eq!(world.sim.sim_relay().probe_calls(), 1, "the relay probe ran once and passed");
    assert!(
        events().refusal_reasons().iter().any(|reason| reason == "gateway.connect_probe"),
        "health.startup.refused names gateway.connect_probe: {:?}",
        events().refusal_reasons()
    );
    assert_unbound_and_serve_unaffected(&world).await;
    world.shutdown().await;
}

// ---------------------------------------------------------------------------
// S-PIG-58
// ---------------------------------------------------------------------------

/// S-PIG-58 (a) — a Route whose Service has no backend (the Sim driver refuses
/// to start the allocation, observed before the request) is bind-valid: the
/// gate waits for no backend and makes no `open_upstream`, `take_receipt` or
/// `authenticate` call, the listener binds, the connect path is Ready, no
/// unavailable cause is reported, and the bound listener serves 503.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn a_frontend_with_no_backend_is_bind_valid_and_the_listener_serves_503() {
    let mut world = World::boot(Spec {
        driver_refuses_start: true,
        spy: true,
        frontend_on_loopback: true,
        ..Spec::enabled(154)
    })
    .await;

    let status = world.promoted().await;
    world.assert_no_backend_programmed().await;
    world.assert_gate_made_no_service_connection();

    let row = status.application.as_ref().expect("an application row");
    assert!(row.unavailable.is_none(), "no unavailable cause once promoted: {status:?}");
    assert!(
        matches!(row.connect_path, GatewayConnectPathStatusBody::Ready),
        "the connect path is Ready: {status:?}"
    );
    assert!(
        matches!(row.gateway_identity, GatewayIdentityStatusBody::Current { .. }),
        "the Gateway Identity is Current: {status:?}"
    );
    assert!(world.listening(), "TCP/443 is bound on the configured address");
    let clock_before = world.clock.now();
    assert!(
        world.public_response().await.starts_with(b"HTTP/1.1 503"),
        "the listener serves the NoBackend 503"
    );
    assert_eq!(
        world.clock.now(),
        clock_before,
        "the request resolves on the refused loopback frontend and never waits"
    );
    world.shutdown().await;
}

/// S-PIG-58 (b) — route activation does not depend on backend authentication
/// (R-65). The Service is Running and its backend is programmed, but the Sim
/// exact-peer client presents no peer, so the selected backend's mTLS fails.
/// The gate makes no connection to the Route's Service: it makes no
/// `open_upstream`, `take_receipt` or `authenticate` call, so the listener
/// binds, TCP/443 is bound, the status shows no gateway failure cause and
/// `health.startup.refused` is not emitted before the first public request.
/// That request returns exactly one 502, with one `open_upstream`, one
/// `take_receipt`, one `authenticate` and no retry, and leaves no intent or
/// receipt.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn a_selected_backend_that_refuses_mtls_is_bind_valid_and_its_first_request_is_one_502() {
    let mut world =
        World::boot(Spec { spy: true, frontend_on_loopback: true, ..Spec::enabled(167) }).await;
    world.deploy_service().await;
    world.wait_for_programmed_backend().await;
    // The Sim does not rewrite the destination, so the dial goes to the
    // Service's own loopback VIP: serve it, so the connect succeeds and the
    // refusal under test is the exact-peer mTLS one.
    let vips = world.dataplane.service_vip_keys();
    assert_eq!(vips.len(), 1, "one Service frontend is programmed: {vips:?}");
    let _frontend = FrontendDrain::start(SocketAddr::V4(SocketAddrV4::new(vips[0], SERVICE_PORT)));
    world.declare_route("/").await;

    let status = world
        .wait_status("the listener to bind", TICK, |status| {
            status.listener_bound
                && status.application.as_ref().is_some_and(|row| {
                    matches!(row.listener, GatewayListenerStatusBody::Bound { .. })
                })
        })
        .await;

    let row = status.application.as_ref().expect("an application row");
    assert!(row.unavailable.is_none(), "no gateway failure cause: {status:?}");
    assert!(
        matches!(row.connect_path, GatewayConnectPathStatusBody::Ready),
        "the connect path is Ready: {status:?}"
    );
    assert!(world.listening(), "TCP/443 is bound on the configured address");
    assert!(
        events().refusal_reasons().is_empty(),
        "no health.startup.refused: {:?}",
        events().refusal_reasons()
    );
    world.assert_gate_made_no_service_connection();

    let response = world.public_response().await;
    assert!(
        response.starts_with(b"HTTP/1.1 502"),
        "the first request is a 502, got {:?}",
        String::from_utf8_lossy(&response[..response.len().min(40)])
    );
    assert_eq!(
        response.windows(9).filter(|window| *window == b"HTTP/1.1 ").count(),
        1,
        "exactly one response"
    );
    assert_eq!(world.opens(), 1, "one open_upstream, no retry");
    assert_eq!(world.receipt_takes(), 1, "one take_receipt, no second attempt");
    assert_eq!(world.authenticate_calls(), 1, "one authenticate, no second attempt");
    assert_eq!(
        (
            world.connect.live_intent_count().ok(),
            world.connect.live_receipts().ok().map(|r| r.len())
        ),
        (Some(0), Some(0)),
        "no intent or receipt outlives the request"
    );
    world.shutdown().await;
}

/// S-PIG-58 (c) — behind the same refusing backend a Route replacement is
/// promoted, and the prior generation drains and retires exactly as it does
/// without a backend: the listener stays bound, nothing is staged or draining,
/// no failure cause is reported, neither gate made an `open_upstream`,
/// `take_receipt` or `authenticate` call, and allocation, Service and
/// hydration advance as on an unrefused node.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn a_route_replacement_behind_a_backend_that_refuses_mtls_is_promoted_and_drains_the_prior_generation()
 {
    let mut world = World::boot(Spec { spy: true, ..Spec::enabled(168) }).await;
    world.deploy_service().await;
    let before = world.wait_for_programmed_backend().await;
    world.declare_route("/").await;
    let first = world
        .wait_status("the first generation to be current", TICK, |status| {
            status.application.as_ref().is_some_and(|row| row.current.is_some())
        })
        .await;
    let first_generation = first
        .application
        .and_then(|row| row.current)
        .map(|current| current.application_generation)
        .expect("a current generation");

    let replaced = world.declare_route("/v2").await;
    assert_eq!(
        replaced.outcome,
        overdrive_core::public_ingress::RouteApplyOutcome::Replaced,
        "the same Route ID with new fields is a replacement"
    );
    let next = world
        .wait_status("the replacement to be current", TICK, |status| {
            status
                .application
                .as_ref()
                .and_then(|row| row.current.as_ref())
                .is_some_and(|current| current.application_generation != first_generation)
        })
        .await;

    let row = next.application.as_ref().expect("an application row");
    assert!(next.listener_bound, "the listener stays bound across the replacement");
    assert!(row.unavailable.is_none(), "no failure cause: {next:?}");
    assert!(row.staged.is_none(), "nothing is staged: {next:?}");
    assert!(row.draining.is_empty(), "the prior generation retired: {next:?}");
    let after = world.progress().await;
    assert!(after.allocation_running && after.backends_hydrated >= before.backends_hydrated);
    assert!(after.operator_listener_answers, "the operator listener advances regardless");
    world.assert_gate_made_no_service_connection();
    world.shutdown().await;
}

// ---------------------------------------------------------------------------
// S-PIG-60 (I arm)
// ---------------------------------------------------------------------------

/// S-PIG-60 (I arm) — enabled boot runs the relay probe exactly once and
/// before the connect probe (inside the gate task); graceful shutdown runs
/// stage `gateway` before `mtls_worker_owner`, inside the one drain, and
/// releases the bound address.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn enabled_boot_probes_the_relay_before_the_connect_path_and_the_gateway_stage_runs_first() {
    let mut spec = Spec::enabled(155);
    spec.spy = true;
    let mut world = World::boot(spec).await;
    world.promoted().await;

    assert_eq!(world.sim.sim_relay().probe_calls(), 1, "the relay probe runs once per boot");
    assert_eq!(
        *world.relay_probes_seen_by_connect_probe.lock().expect("probe log lock"),
        vec![1],
        "the connect probe ran once, after the relay probe had run (G3 order)"
    );
    let stages_before = events().shutdown_stages().len();

    let started = Instant::now();
    world.shutdown().await;
    assert!(started.elapsed() < Duration::from_secs(10), "shutdown completes inside the bound");

    let stages = events().shutdown_stages();
    let ours = &stages[stages_before..];
    let position = |name: &str| ours.iter().position(|stage| stage == name);
    let gateway = position("gateway").expect("the gateway stage ran");
    let worker = position("mtls_worker_owner").expect("the mtls worker stage ran");
    assert_eq!(gateway, 0, "the gateway stage is the first shutdown stage: {ours:?}");
    assert!(gateway < worker, "stage gateway precedes mtls_worker_owner: {ours:?}");
    assert!(!world.listening(), "the public address is released after shutdown");
    assert_eq!(
        world.connect.live_intent_count().expect("live intents"),
        0,
        "shutdown leaves no connect residue"
    );
}

// ---------------------------------------------------------------------------
// S-PIG-61
// ---------------------------------------------------------------------------

/// S-PIG-61 (gated) — a gated gateway never requests serve shutdown: with the
/// relay probe refused, no shutdown request arrives while the node keeps
/// ticking, and a requested shutdown completes inside the 10 s fail-stop
/// bound with the gateway stage in the same 5 s drain.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn a_gated_gateway_never_requests_serve_shutdown() {
    let mut spec = Spec::enabled(156);
    spec.relay_lie = Some(SubstrateLie::UlpAbsent);
    let mut world = World::boot(spec).await;
    world.deploy_service().await;
    world.declare_route("/").await;
    world.wait_status("the gated status", TICK, connect_path_unavailable).await;

    let mut handle = world.handle.take().expect("the server handle");
    let outcome = {
        let request = tokio::time::timeout(Duration::from_millis(500), handle.shutdown_requested());
        let mut request = std::pin::pin!(request);
        loop {
            tokio::select! {
                biased;
                outcome = &mut request => break outcome,
                () = tokio::time::sleep(Duration::from_millis(20)) => world.clock.tick(TICK),
            }
        }
    };
    assert!(outcome.is_err(), "a gated gateway requests no serve shutdown: {outcome:?}");
    world.handle = Some(handle);

    let started = Instant::now();
    world.shutdown().await;
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "shutdown completes inside the unchanged 10 s fail-stop bound"
    );
}

// ---------------------------------------------------------------------------
// S-PIG-62
// ---------------------------------------------------------------------------

/// S-PIG-62 — Allocation `Running`, the Service's hydration into the dataplane,
/// any `Stable` claim and the operator listener advance identically with the
/// gateway disabled, enabled or gated (boundary 3).
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn the_workload_surface_advances_identically_disabled_enabled_and_gated() {
    let mut progress = Vec::new();
    for (label, spec) in [
        ("disabled", Spec { enabled: false, ..Spec::enabled(158) }),
        ("enabled", Spec::enabled(159)),
        ("gated", Spec { relay_lie: Some(SubstrateLie::UlpAbsent), ..Spec::enabled(160) }),
    ] {
        let mut world = World::boot(spec).await;
        world.deploy_service().await;
        let deadline = Instant::now() + WAIT;
        let reached = loop {
            let observed = world.progress().await;
            if observed.allocation_running && observed.backends_hydrated > 0 {
                break observed;
            }
            assert!(
                Instant::now() < deadline,
                "{label}: the allocation did not reach Running and hydrate: {observed:?}"
            );
            world.settle(TICK).await;
        };
        progress.push((label, reached));
        world.shutdown().await;
    }

    let (_, baseline) = &progress[0];
    for (label, observed) in &progress[1..] {
        assert_eq!(observed, baseline, "{label} advances exactly as the disabled node does");
    }
    assert!(baseline.operator_listener_answers, "the operator listener answers");
}

// ---------------------------------------------------------------------------
// S-PIG-74 (b), (c); (a) is the S-PIG-57 (I) arm
// ---------------------------------------------------------------------------

/// The shared body of a boundary-2 case: the node boots, the Route is
/// declared, and whatever blocked the gate leaves TCP/443 unbound with the
/// redacted connect-path cause, serve unaffected.
async fn assert_gate_unavailable(
    mut world: World,
    cause: fn(&GatewayStatusResponse) -> bool,
    step: Duration,
) {
    world.deploy_service().await;
    world.declare_route("/").await;
    let status = world.wait_status("the unavailable cause", step, cause).await;
    assert!(!status.listener_bound, "the listener is unbound: {status:?}");
    assert_unbound_and_serve_unaffected(&world).await;
    world.shutdown().await;
}

/// S-PIG-74 (b) — the Gateway Identity never becomes Current (the CA refuses
/// the gateway's SVID): TCP/443 stays unbound with the identity cause, and the
/// connect-path reasons are not emitted.
///
/// The clock advances one second per poll so the gate's identity deadline
/// elapses on the injected clock.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn a_gateway_identity_that_never_becomes_current_keeps_the_public_port_unbound() {
    let mut spec = Spec::enabled(163);
    spec.refuse_svid = true;
    let world = World::boot(spec).await;
    assert_gate_unavailable(
        world,
        |status| {
            unavailable_is(status, |cause| {
                matches!(cause, GatewayApplicationUnavailableCauseBody::GatewayIdentityUnusable)
            })
        },
        Duration::from_secs(1),
    )
    .await;
    let reasons = events().refusal_reasons();
    assert!(
        !reasons
            .iter()
            .any(|reason| reason == "gateway.connect_probe" || reason == "gateway.relay_probe"),
        "an identity gate emits no probe reason: {reasons:?}"
    );
}

/// S-PIG-74 (c) — the probe does not complete before its deadline
/// (`ProbeTimeout` at the probe operation).
///
/// The stimulus is the typed `ProbeTimeout` result at the existing `probe`
/// operation. A held gate (Sim `HoldConnectProbe`, which parks at an awaited
/// operation the gate owns, DG-9, R-65) is Sim test support and is not this
/// stimulus: the once-per-boot sentinel probe is the connect dataplane's own
/// `probe`, whose typed result already reaches the gate outcome.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn a_probe_that_times_out_keeps_the_public_port_unbound() {
    let mut spec = Spec::enabled(164);
    spec.probe_fault = Some(GatewayConnectDataplaneError::ProbeTimeout);
    let world = World::boot(spec).await;
    assert_gate_unavailable(world, connect_path_unavailable, TICK).await;
    assert!(
        events().refusal_reasons().iter().any(|reason| reason == "gateway.connect_probe"),
        "health.startup.refused names gateway.connect_probe: {:?}",
        events().refusal_reasons()
    );
}

// ---------------------------------------------------------------------------
// S-PIG-100 (enabled arms)
// ---------------------------------------------------------------------------

/// S-PIG-100 (lying relay) — a refused relay probe keeps TCP/443 unbound with
/// reason `gateway.relay_probe` and status `ConnectPathUnavailable`; the
/// connect probe never runs; allocation `Running`, hydration and the operator
/// listener advance as on an honest node.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn a_refused_relay_probe_keeps_the_public_port_unbound_and_the_connect_probe_unrun() {
    let mut spec = Spec::enabled(165);
    spec.relay_lie = Some(SubstrateLie::UlpAbsent);
    spec.spy = true;
    let mut world = World::boot(spec).await;
    world.deploy_service().await;
    world.declare_route("/").await;

    let status = world.wait_status("the relay gate", TICK, connect_path_unavailable).await;

    assert!(!status.listener_bound, "the listener is unbound: {status:?}");
    assert!(world.refused(), "TCP/443 is unbound");
    assert!(
        events().refusal_reasons().iter().any(|reason| reason == "gateway.relay_probe"),
        "health.startup.refused names gateway.relay_probe: {:?}",
        events().refusal_reasons()
    );
    assert_eq!(world.sim.sim_relay().probe_calls(), 1, "the relay probe ran once");
    assert!(
        world.relay_probes_seen_by_connect_probe.lock().expect("probe log lock").is_empty(),
        "the connect probe never runs behind a refused relay probe"
    );
    let progress = loop {
        let observed = world.progress().await;
        if observed.allocation_running && observed.backends_hydrated > 0 {
            break observed;
        }
        world.settle(TICK).await;
    };
    assert!(progress.operator_listener_answers, "the operator listener advances regardless");
    world.shutdown().await;
}

/// S-PIG-100 (once per boot) — the relay probe runs once per boot: replacing
/// the Route after promotion stages and promotes a later generation without
/// another relay probe or another connect probe.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn the_relay_probe_runs_once_per_boot_across_a_route_replacement() {
    let mut spec = Spec::enabled(166);
    spec.spy = true;
    let mut world = World::boot(spec).await;
    let first = world.promoted().await;
    let first_generation = first
        .application
        .and_then(|row| row.current)
        .map(|current| current.application_generation)
        .expect("a current generation");

    let replaced = world.declare_route("/v2").await;
    assert_eq!(
        replaced.outcome,
        overdrive_core::public_ingress::RouteApplyOutcome::Replaced,
        "the same Route ID with new fields is a replacement"
    );
    let next = world
        .wait_status("the replacement to be current", TICK, |status| {
            status
                .application
                .as_ref()
                .and_then(|row| row.current.as_ref())
                .is_some_and(|current| current.application_generation != first_generation)
        })
        .await;

    assert!(next.listener_bound, "the listener stays bound across the replacement");
    assert_eq!(
        world.sim.sim_relay().probe_calls(),
        1,
        "no second relay probe for a later generation"
    );
    assert_eq!(
        *world.relay_probes_seen_by_connect_probe.lock().expect("probe log lock"),
        vec![1],
        "no second connect probe either: the one probe ran after the one relay probe"
    );
    world.shutdown().await;
}

// ---------------------------------------------------------------------------
// S-PIG-140 and S-PIG-141 (R-67): Route command failures are typed, never
// flattened
// ---------------------------------------------------------------------------

/// The store key of the singleton Route-set record. The gateway crate keeps
/// the constant crate-private; the stimulus below is addressed to the key the
/// production owner reads, so a rename there fails these bodies at their
/// precondition.
const ROUTE_SET_KEY: &[u8] = b"public-ingress/route-set";
/// A syntactically valid Route ID that is not the declared one.
const OTHER_ROUTE_ID: &str = "other-route";

/// One operator answer to a Route command: the status and the JSON body.
#[derive(Debug)]
struct RouteReply {
    status: u16,
    body: serde_json::Value,
}

impl RouteReply {
    fn error_kind(&self) -> Option<&str> {
        self.body.get("error").and_then(serde_json::Value::as_str)
    }

    /// The reply is the typed error `(status, kind, message)` with no field
    /// and no cause text.
    fn assert_typed(&self, status: u16, kind: &str, message: &str, what: &str) {
        assert_eq!(self.status, status, "{what}: status; body {}", self.body);
        assert_eq!(self.error_kind(), Some(kind), "{what}: error kind; body {}", self.body);
        assert_eq!(
            self.body.get("message").and_then(serde_json::Value::as_str),
            Some(message),
            "{what}: the fixed message, no cause text"
        );
        assert!(
            self.body.get("field").is_none_or(serde_json::Value::is_null),
            "{what}: no field; body {}",
            self.body
        );
    }
}

fn route_input(path: &str) -> PublicRouteInput {
    PublicRouteInput {
        id: ROUTE_ID.to_owned(),
        hostname: HOSTNAME.to_owned(),
        path: path.to_owned(),
        path_match: "segment_prefix".to_owned(),
        certified_key: CERTIFIED_KEY_ID.to_owned(),
        target: ServiceListenerReferenceInput {
            service: SERVICE_ID.to_owned(),
            port: SERVICE_PORT,
            protocol: "tcp".to_owned(),
        },
    }
}

impl World {
    async fn try_route_post(&self, path: &str) -> Result<RouteReply, reqwest::Error> {
        let response = self
            .client
            .post(format!("{}/v1/routes", self.base))
            .json(&route_input(path))
            .send()
            .await?;
        Self::reply(response).await
    }

    async fn try_route_delete(&self, id: &str) -> Result<RouteReply, reqwest::Error> {
        let response = self.client.delete(format!("{}/v1/routes/{id}", self.base)).send().await?;
        Self::reply(response).await
    }

    async fn reply(response: reqwest::Response) -> Result<RouteReply, reqwest::Error> {
        let status = response.status().as_u16();
        let body = response.json().await.unwrap_or(serde_json::Value::Null);
        Ok(RouteReply { status, body })
    }

    async fn route_post(&self, path: &str) -> RouteReply {
        self.try_route_post(path).await.expect("POST /v1/routes")
    }

    async fn route_delete(&self, id: &str) -> RouteReply {
        self.try_route_delete(id).await.expect("DELETE a Route by id")
    }

    /// The bytes the intent store holds at the Route-set key, read through the
    /// `IntentStore` port.
    async fn stored_route_set(&self) -> Option<Vec<u8>> {
        self.intent
            .get(ROUTE_SET_KEY)
            .await
            .expect("read the Route-set key")
            .map(|bytes| bytes.as_ref().to_vec())
    }

    async fn store_route_set_bytes(&self, bytes: &[u8]) {
        self.intent.put(ROUTE_SET_KEY, bytes).await.expect("write the Route-set key");
    }

    /// The Route-set record fault of S-PIG-140: `bytes` are what the store
    /// returns at the Route-set key. Both methods answer the typed
    /// `route_decode` 500, the store is left as it was, the operator surface
    /// still answers, and after the slot is restored to a valid empty set a
    /// declare succeeds: the fault did not wedge the owner.
    async fn assert_decode_fault_is_typed_and_does_not_wedge(&self, bytes: &[u8], arm: &str) {
        self.store_route_set_bytes(bytes).await;

        let post = self.route_post("/").await;
        post.assert_typed(
            500,
            "route_decode",
            "stored public Route is undecodable",
            &format!("{arm}: POST"),
        );
        let delete = self.route_delete(ROUTE_ID).await;
        delete.assert_typed(
            500,
            "route_decode",
            "stored public Route is undecodable",
            &format!("{arm}: DELETE"),
        );

        assert_eq!(
            self.stored_route_set().await.as_deref(),
            Some(bytes),
            "{arm}: neither command rewrote the stored bytes"
        );
        let status = self.status().await;
        assert!(status.enabled, "{arm}: the operator surface still answers");

        let empty =
            PublicRouteSetV1 { route: None }.archive_for_store().expect("archive empty set");
        self.store_route_set_bytes(empty.as_slice()).await;
        let declared = self.try_route_post("/").await.expect("POST after restore");
        assert_eq!(
            declared.status, 201,
            "{arm}: a declare after the restore succeeds; {declared:?}"
        );
        assert_eq!(
            declared.body.get("outcome").and_then(serde_json::Value::as_str),
            Some("Declared"),
            "{arm}: the owner is not wedged"
        );
    }
}

/// S-PIG-140 (R-67), garbage arm: bytes at the Route-set key that are no
/// envelope at all reach the operator as the typed 500 `route_decode` on both
/// `POST /v1/routes` and `DELETE /v1/routes/{id}`, not as `internal` or any
/// other class; the stored bytes are unchanged; the owner is not wedged.
///
/// The fault enters through the `IntentStore` driven port on the booted
/// composition's own store; the Route handlers and the Route owner are the
/// production ones. `PublicRouteSetError::Encode` is reachable through no
/// production port, so no scenario drives it (R-67 no-obligation ruling).
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER route-set-error-mapping"]
async fn an_undecodable_stored_route_is_a_typed_500_route_decode_on_post_and_delete() {
    let mut world = World::boot(Spec::enabled(169)).await;
    world
        .assert_decode_fault_is_typed_and_does_not_wedge(
            b"\x00 not an rkyv envelope \xff\xfe",
            "garbage bytes",
        )
        .await;
    world.shutdown().await;
}

/// The bytes of a well-formed `PublicRouteSetV1` envelope whose Route holds a
/// hostname no Route validation accepts: a valid archive with the hostname's
/// octets replaced by an equally long name containing underscores.
fn envelope_holding_an_invalid_route() -> Vec<u8> {
    let route = Route::from_submit(route_input("/")).expect("a valid Route");
    let mut bytes = PublicRouteSetV1 { route: Some(route) }
        .archive_for_store()
        .expect("archive the set")
        .as_slice()
        .to_vec();
    let valid = HOSTNAME.as_bytes();
    let invalid = HOSTNAME.replace(['.'], "_");
    assert_eq!(invalid.len(), valid.len());
    let at: Vec<usize> = bytes
        .windows(valid.len())
        .enumerate()
        .filter_map(|(index, window)| (window == valid).then_some(index))
        .collect();
    assert_eq!(at.len(), 1, "the hostname appears once in the archived Route");
    bytes[at[0]..at[0] + valid.len()].copy_from_slice(invalid.as_bytes());
    // The decode validates the stored Route (R-67 (4)): an envelope that is
    // well formed and holds a Route no constructor accepts is refused with
    // `RouteCodecError::InvalidRoute(RouteValidationError::Hostname(_))`, and
    // never with an `Envelope` failure. The two facts together prove the
    // patched bytes are a well-formed envelope holding an invalid Route. The
    // variant is read through `Debug` so this file compiles against the
    // decode's present return type as well as its R-67 return type.
    let refusal = PublicRouteSetV1::from_store_bytes(
        &bytes,
        std::path::Path::new("<fixture>"),
        Some("public-ingress/route-set"),
    )
    .expect_err("the decode refuses a Route no constructor accepts (R-67 (4))");
    assert!(
        format!("{refusal:?}").starts_with("InvalidRoute(Hostname("),
        "the patched bytes are a well-formed envelope holding an invalid hostname, got {refusal:?}"
    );
    bytes
}

/// S-PIG-140 (R-67), invalid-Route arm: a well-formed envelope whose Route
/// fails Route validation is the same typed 500 `route_decode` on both
/// methods, with the same oracle as the garbage arm. The stored Route is
/// validated by the decode itself (R-67 (4)).
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER route-set-error-mapping"]
async fn a_well_formed_envelope_holding_an_invalid_route_is_a_typed_500_route_decode() {
    let mut world = World::boot(Spec::enabled(170)).await;
    world
        .assert_decode_fault_is_typed_and_does_not_wedge(
            &envelope_holding_an_invalid_route(),
            "envelope holding an invalid Route",
        )
        .await;
    world.shutdown().await;
}

/// A client that issues one Route command after another against the operator
/// listener until told to stop, so a command is always queued or awaiting its
/// reply while the node shuts down. `post` selects `POST /v1/routes` (a
/// declare of the stored Route, which never conflicts) or `DELETE` of another
/// Route's ID (which never writes). It returns every answer it received;
/// transport errors (the listener already closed) are not answers.
fn keep_a_route_command_in_flight(
    client: reqwest::Client,
    base: String,
    post: bool,
    answered: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<Vec<RouteReply>> {
    tokio::spawn(async move {
        let mut answers = Vec::new();
        while !stop.load(Ordering::SeqCst) {
            let sent = if post {
                client.post(format!("{base}/v1/routes")).json(&route_input("/")).send().await
            } else {
                client.delete(format!("{base}/v1/routes/{OTHER_ROUTE_ID}")).send().await
            };
            match sent {
                Ok(response) => {
                    answers.push(World::reply(response).await.expect("read the reply"));
                    answered.fetch_add(1, Ordering::SeqCst);
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(1)).await,
            }
        }
        answers
    })
}

/// S-PIG-141 (R-67), shutdown arm: a Route command queued, awaiting its reply,
/// or sent after the gateway stage of the graceful shutdown ended the Route
/// owner, while the operator listener still drains, answers the typed 503
/// `gateway_owner_unavailable`; the Route set in the store is unchanged.
///
/// One command of each method is kept in flight continuously across the
/// shutdown (R-67 (5)): the window between the end of the Route owner and the
/// close of the operator listener is not bounded in width, so sampling it
/// would be a race, whereas a command that is queued or awaiting its reply
/// when the owner is cancelled is answered `OwnerUnavailable` by construction.
/// Every answer before the owner ends is a success or a Route conflict (the
/// DELETE names another Route, so it never writes); the first other answer of
/// each method must be the 503, and each method must have received it.
///
/// The refused-boot arm of S-PIG-141 takes a no-obligation ruling (R-67 (5)):
/// every refusal after G1 precedes the spawn of the operator listener's
/// server task, so no HTTP request can observe the dropped receiver.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER route-set-error-mapping"]
async fn a_route_owner_that_ended_in_shutdown_answers_503_gateway_owner_unavailable_on_both_methods()
 {
    let mut world = World::boot(Spec::enabled(171)).await;
    world.declare_route("/").await;
    let before = world.stored_route_set().await;
    assert!(before.is_some(), "the declared Route is stored");

    let stop = Arc::new(AtomicBool::new(false));
    let (posts_answered, deletes_answered) =
        (Arc::new(AtomicU32::new(0)), Arc::new(AtomicU32::new(0)));
    let poster = keep_a_route_command_in_flight(
        world.client.clone(),
        world.base.clone(),
        true,
        Arc::clone(&posts_answered),
        Arc::clone(&stop),
    );
    let deleter = keep_a_route_command_in_flight(
        world.client.clone(),
        world.base.clone(),
        false,
        Arc::clone(&deletes_answered),
        Arc::clone(&stop),
    );
    // Both clients are live (each has been answered once) before the shutdown
    // starts, so each has a command in flight from then on.
    let live_deadline = Instant::now() + WAIT;
    while posts_answered.load(Ordering::SeqCst) == 0 || deletes_answered.load(Ordering::SeqCst) == 0
    {
        assert!(Instant::now() < live_deadline, "the Route command clients never got an answer");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    let handle = world.handle.take().expect("the server handle");
    let clock = Arc::clone(&world.clock);
    let mut shutdown = std::pin::pin!(handle.shutdown(Duration::from_secs(5)));
    let shutdown_deadline = Instant::now() + WAIT;
    loop {
        assert!(Instant::now() < shutdown_deadline, "the graceful shutdown never completed");
        tokio::select! {
            biased;
            result = &mut shutdown => { result.expect("graceful shutdown"); break; }
            () = tokio::time::sleep(Duration::from_millis(5)) => clock.tick(TICK),
        }
    }
    stop.store(true, Ordering::SeqCst);
    let posts = poster.await.expect("the declare client");
    let deletes = deleter.await.expect("the delete client");

    let message = "public Route owner is unavailable";
    for (what, answers) in [("POST", &posts), ("DELETE", &deletes)] {
        let first_refusal = answers
            .iter()
            .find(|reply| !(reply.status < 300 || reply.status == 409))
            .unwrap_or_else(|| panic!("{what}: no command was refused by the shutdown window"));
        first_refusal.assert_typed(503, "gateway_owner_unavailable", message, what);
        assert!(
            answers
                .iter()
                .skip_while(|reply| reply.status < 300 || reply.status == 409)
                .all(|reply| reply.status == 503),
            "{what}: after the first refusal every answer is the 503; {answers:?}"
        );
    }
    assert_eq!(
        world.stored_route_set().await,
        before,
        "no write reached the intent store after the owner ended"
    );
}
