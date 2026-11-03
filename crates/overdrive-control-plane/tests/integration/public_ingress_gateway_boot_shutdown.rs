//! Public-ingress gateway — boot and shutdown through the ordinary production
//! composition (S-PIG-59, and the disabled arm of S-PIG-100).
//!
//! Every body enters through `run_server(config, adapters)` over the host bundle
//! (`NodeAdapters::host`, NO-OVERRIDE-SEAMS) with the non-VM fixtures'
//! simulations (`SimDataplaneComposer`, `SimMtlsIntercept`, `SimGuestDnsFactory`)
//! and observes the real operator HTTPS surface and the tracing stage events.
//!
//! | Body | Scenario | State |
//! |---|---|---|
//! | `a_disabled_gateway_boots_and_shuts_down_exactly_as_before` | S-PIG-59 (boundary 6) | expected GREEN, no `#[ignore]` |
//! | `a_disabled_gateway_never_probes_the_relay` | S-PIG-100, disabled arm | ignored until `SimGatewayRelay` lands |
//!
//! The enabled arms of S-PIG-02 (I), 57, 58, 60, 61, 62, 74 and 100 boot the
//! gateway (steps G1-G4). They cannot be authored: the enabled composition
//! needs `GatewayAdapters.client_mtls` and boot step G2, whose port
//! (`GatewayClientMtlsFactory`) has no pinned crate placement (feature-delta
//! PROPOSED P-20, recorded in the batch gaps file).
//!
//! # Bind-address table (R-6)
//!
//! A composition that reaches promotion binds its own `127.0.0.0/8` address on
//! TCP/443. This file owns second octet 97; the gateway integration files own
//! 41..=62 and 68..=96, the in-crate drivers 63..=67.
//!
//! | `T` | test |
//! |---|---|
//! | 97 | disabled boot: the configured-but-unused address stays unbound (S-PIG-59) |

#![allow(clippy::doc_markdown, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use overdrive_control_plane::api::ClusterStatus;
use overdrive_core::reconcilers::Reconciler;
use overdrive_core::traits::clock::Clock;
use overdrive_gateway::ports::GatewayRelay;
use overdrive_reconcilers::GatewaySvidLifecycle;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::gateway::SimGatewayRelay;
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
