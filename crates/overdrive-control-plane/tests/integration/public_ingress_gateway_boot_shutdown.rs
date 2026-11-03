//! Public-ingress gateway — boot and shutdown through the ordinary production
//! composition: S-PIG-59 and the disabled arm of S-PIG-100 (over the host
//! bundle, `run_server`), and the enabled-boot arms of S-PIG-02 (I), 54, 57
//! (I), 58, 60 (I), 61, 62, 74 (a)-(d) and 100 (over the supplied-owner
//! `run_server_with_obs_and_driver`, gateway enabled through
//! `ServerConfig.gateway`).
//!
//! Every body enters through the production entry point and observes the real
//! operator HTTPS surface, the tracing stage events and the Sim relay and
//! connect-dataplane transcripts. Faults enter only through existing driven
//! ports: the Sim relay's substrate lie, the Sim connect dataplane's typed
//! `fail_next`, a delegating observation store, and a delegating CA that
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
//! | `a_failing_sentinel_probe_keeps_the_public_port_unbound_and_serve_unaffected` | S-PIG-57 (I) | ignored |
//! | `a_frontend_with_no_backend_is_bind_valid_and_the_listener_serves_503` | S-PIG-58 | ignored |
//! | `enabled_boot_probes_the_relay_before_the_connect_path_and_the_gateway_stage_runs_first` | S-PIG-60 (I) | ignored |
//! | `a_gated_gateway_never_requests_serve_shutdown` | S-PIG-61 | ignored |
//! | `a_gateway_task_failure_never_requests_serve_shutdown` | S-PIG-61 | ignored |
//! | `the_workload_surface_advances_identically_disabled_enabled_and_gated` | S-PIG-62 | ignored |
//! | `an_absent_sentinel_receipt_keeps_the_public_port_unbound` | S-PIG-74 (a) | ignored |
//! | `a_hook_that_does_not_fire_keeps_the_public_port_unbound` | S-PIG-74 (b) | ignored |
//! | `a_gateway_identity_that_never_becomes_current_keeps_the_public_port_unbound` | S-PIG-74 (c) | ignored |
//! | `a_probe_that_times_out_keeps_the_public_port_unbound` | S-PIG-74 (d) | ignored |
//! | `a_refused_relay_probe_keeps_the_public_port_unbound_and_the_connect_probe_unrun` | S-PIG-100 | ignored |
//! | `the_relay_probe_runs_once_per_boot_across_a_route_replacement` | S-PIG-100 | ignored |
//!
//! # Bind-address table (R-6)
//!
//! A composition that reaches promotion binds its own `127.0.0.0/8` address on
//! TCP/443. This file owns second octets 97 and 152..=166; the gateway
//! integration files own 41..=62, 68..=96 and 130..=132, the in-crate drivers
//! 63..=67, the sim acceptance file 120 and 121, the conformance binary
//! 100..=119.
//!
//! | `T` | test |
//! |---|---|
//! | 97 | disabled boot: the configured-but-unused address stays unbound (S-PIG-59) |
//! | 152 | S-PIG-02 / 54 |
//! | 153 | S-PIG-57 |
//! | 154 | S-PIG-58 |
//! | 155 | S-PIG-60 |
//! | 156, 157 | S-PIG-61 (gated, task failure) |
//! | 158, 159, 160 | S-PIG-62 (disabled, enabled, gated) |
//! | 161, 162, 163, 164 | S-PIG-74 (a), (b), (c), (d) |
//! | 165, 166 | S-PIG-100 (refused relay probe; once per boot) |

#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_const_for_fn,
    reason = "CONTRACT_SHAPE metadata; fixture preconditions are asserted"
)]

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
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
use overdrive_core::reconcilers::Reconciler;
use overdrive_core::traits::ca::{
    Ca, CaError, CaFactory, IntermediateHandle, RootCaHandle, SvidMaterial, SvidRequest,
    TrustBundle,
};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::entropy::Entropy;
use overdrive_gateway::ports::{
    GatewayConnectCleanup, GatewayConnectCleanupSweep, GatewayConnectDataplane,
    GatewayConnectDataplaneError, GatewayConnectIntent, GatewayRelay, GatewaySelectionReceipt,
    PreparedUpstream,
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
use serial_test::serial;
use tempfile::TempDir;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

/// The address a disabled node would have bound had the gateway been enabled
/// (R-6, octet 97): nothing may listen on it.

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
#[ignore = "pending DELIVER kernel-relay-sim-adapter"]
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
/// Ignore key of the boot arms.
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
/// connect probe, how many relay probes had already run (the G3 order).
struct ProbeOrderSpy {
    inner: Arc<SimGatewayConnectDataplane>,
    relay: Arc<SimGatewayRelay>,
    relay_probes_seen_by_connect_probe: Arc<Mutex<Vec<u32>>>,
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
        self.inner.open_upstream(key)
    }

    fn take_receipt(
        &self,
        intent: &GatewayConnectIntent,
    ) -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError> {
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

/// What a world is built with. Every field is a choice at an existing driven
/// port or in `ServerConfig`; the defaults are the healthy enabled node.
struct Spec {
    octet: u8,
    enabled: bool,
    relay_lie: Option<SubstrateLie>,
    probe_fault: Option<GatewayConnectDataplaneError>,
    refuse_svid: bool,
    spy: bool,
}

impl Spec {
    fn enabled(octet: u8) -> Self {
        Self {
            octet,
            enabled: true,
            relay_lie: None,
            probe_fault: None,
            refuse_svid: false,
            spy: false,
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
    status_fault: Arc<AtomicBool>,
    relay_probes_seen_by_connect_probe: Arc<Mutex<Vec<u32>>>,
    handle: Option<ServerHandle>,
    client: reqwest::Client,
    base: String,
}

impl World {
    async fn boot(spec: Spec) -> Self {
        let events = events();
        let _ = events;
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
        let capability: Arc<dyn GatewayConnectDataplane> = if spec.spy {
            Arc::new(ProbeOrderSpy {
                inner: Arc::clone(&connect),
                relay: Arc::clone(sim.sim_relay()),
                relay_probes_seen_by_connect_probe: Arc::clone(&probe_log),
            })
        } else {
            Arc::clone(&connect) as _
        };

        let mut boot: BootAdapters = super::node_adapters::boot_adapters(&config);
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
            client_mtls: sim.client_mtls(),
            relay: sim.relay(),
        };
        if spec.refuse_svid {
            boot.ca.factory = Arc::new(RefusingSvidCaFactory);
        }
        let status_fault = Arc::new(AtomicBool::new(false));
        boot.observation = Arc::new(super::status_fault_store::StatusReadFaultStore {
            inner: Arc::clone(&boot.observation),
            armed: Arc::clone(&status_fault),
        });
        let driver: Arc<dyn Driver> =
            Arc::new(SimDriver::with_clock(DriverType::Vm, Arc::clone(&clock_port)));
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
            status_fault,
            relay_probes_seen_by_connect_probe: probe_log,
            handle: Some(handle),
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
/// The Service is admitted but the clock is not advanced, so no allocation
/// starts and `update_service` is never issued: the consuming-port outcome is
/// `NoBackend`.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER consuming-port-unavailable-outcome"]
async fn a_service_with_no_backend_is_answered_503_twice_with_an_empty_second_delta() {
    let mut world = World::boot(Spec::enabled(152)).await;
    world.promoted().await;

    let first = world.public_response().await;
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

/// S-PIG-57 (I arm) — a sentinel probe that fails keeps TCP/443 unbound,
/// status names the redacted cause, the startup-refused event carries reason
/// `gateway.connect_probe`, and serve and workloads are unaffected.
///
/// The fault is the Sim connect dataplane's typed `fail_next` at the probe
/// operation, the existing driven-port result.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER gateway-boot-and-shutdown"]
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

/// S-PIG-58 — an actual frontend with no backend is bind-valid: the gate
/// waits for no backend, the listener binds, the connect path is Ready, no
/// unavailable cause is reported, and the bound listener serves 503.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER gateway-boot-and-shutdown"]
async fn a_frontend_with_no_backend_is_bind_valid_and_the_listener_serves_503() {
    let mut world = World::boot(Spec::enabled(154)).await;

    let status = world.promoted().await;

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
    assert!(
        world.public_response().await.starts_with(b"HTTP/1.1 503"),
        "the listener serves the NoBackend 503"
    );
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
#[ignore = "pending DELIVER gateway-boot-and-shutdown"]
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
#[ignore = "pending DELIVER gateway-boot-and-shutdown"]
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

/// S-PIG-61 (task failure) — `GAP-C4-1`: the DESIGN pins no driven-port fault
/// that fails the gateway task itself (a failed gate is not a failed task), so
/// this body exercises the closest pinned stimulus: the status-read fault on
/// the observation store the gateway writes its status row through, armed
/// after promotion. Serve shutdown is still never requested.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER gateway-boot-and-shutdown"]
async fn a_gateway_task_failure_never_requests_serve_shutdown() {
    let mut world = World::boot(Spec::enabled(157)).await;
    world.promoted().await;
    world.status_fault.store(true, Ordering::SeqCst);
    for _ in 0..20 {
        world.settle(TICK).await;
    }

    let mut handle = world.handle.take().expect("the server handle");
    let outcome =
        tokio::time::timeout(Duration::from_millis(500), handle.shutdown_requested()).await;
    assert!(outcome.is_err(), "no serve shutdown is requested: {outcome:?}");
    world.handle = Some(handle);
    world.status_fault.store(false, Ordering::SeqCst);
    world.shutdown().await;
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
#[ignore = "pending DELIVER gateway-boot-and-shutdown"]
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
// S-PIG-74 (a)-(d)
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

/// S-PIG-74 (a) — the sentinel connect completes but no receipt is found:
/// `ReceiptMissing` at the probe operation.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER gateway-boot-and-shutdown"]
async fn an_absent_sentinel_receipt_keeps_the_public_port_unbound() {
    let mut spec = Spec::enabled(161);
    spec.probe_fault = Some(GatewayConnectDataplaneError::ReceiptMissing);
    let world = World::boot(spec).await;
    assert_gate_unavailable(world, connect_path_unavailable, TICK).await;
    assert!(
        events().refusal_reasons().iter().any(|reason| reason == "gateway.connect_probe"),
        "health.startup.refused names gateway.connect_probe: {:?}",
        events().refusal_reasons()
    );
}

/// S-PIG-74 (b) — the hook does not fire: the registered probe yields no
/// capability result (`Unavailable` at the probe operation).
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER gateway-boot-and-shutdown"]
async fn a_hook_that_does_not_fire_keeps_the_public_port_unbound() {
    let mut spec = Spec::enabled(162);
    spec.probe_fault = Some(GatewayConnectDataplaneError::Unavailable);
    let world = World::boot(spec).await;
    assert_gate_unavailable(world, connect_path_unavailable, TICK).await;
    assert!(
        events().refusal_reasons().iter().any(|reason| reason == "gateway.connect_probe"),
        "health.startup.refused names gateway.connect_probe: {:?}",
        events().refusal_reasons()
    );
}

/// S-PIG-74 (c) — the Gateway Identity never becomes Current (the CA refuses
/// the gateway's SVID): TCP/443 stays unbound with the identity cause, and the
/// connect-path reasons are not emitted.
///
/// The clock advances one second per poll so the gate's identity deadline
/// elapses on the injected clock.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER gateway-boot-and-shutdown"]
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

/// S-PIG-74 (d) — the probe does not complete before its deadline
/// (`ProbeTimeout` at the probe operation).
///
/// `GAP-C4-2`: the DESIGN names `HoldConnectProbe` plus a clock advance, but a
/// hold parks only `Selected` probes at `authenticate` and the sentinel probe
/// is `NoBackend`; the typed `ProbeTimeout` result at the probe operation is
/// the existing-port stimulus that reaches the same gate outcome.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER gateway-boot-and-shutdown"]
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
#[ignore = "pending DELIVER gateway-boot-and-shutdown"]
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
#[ignore = "pending DELIVER gateway-boot-and-shutdown"]
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
