//! Integration tests for the `run_server` driving port (ADR-0008
//! transport, ADR-0010 TLS bootstrap, step 02-02).
//!
//! Each `#[test]` drives the `run_server` public API and asserts
//! observable outcomes at the real HTTP/TLS boundary using `reqwest` as
//! an out-of-process client:
//!
//! * `run_server` binds on `127.0.0.1:0` (ephemeral port), reports the
//!   actually-bound address back through an `axum_server::Handle`, and
//!   serves TLS over HTTP/2 with ALPN `h2, http/1.1`.
//! * A `reqwest::Client` trusting the minted CA performs a real
//!   TLS 1.3 and HTTP/2 handshake against the server and receives
//!   HTTP 200 for every one of the five ADR-0008 endpoint paths
//!   from the stub router.
//! * ALPN negotiation produces HTTP/2 (not HTTP/1.1).
//! * A `CancellationToken` triggers `graceful_shutdown`; an in-flight
//!   request that began before cancellation still completes with 200.
//!
//! All tests run the server in a Tokio task and invoke reqwest against
//! it — real sockets, real TLS handshake, real HTTP parsing. This is
//! Tier 3 real-network integration per `.claude/rules/testing.md`.
//!
//! The graceful-shutdown failure body (GH #295 S-ND295-54) boots the
//! `run_server_with_obs_and_driver` composition instead, so its fault can
//! enter through the required `ServerConfig.mtls_intercept` port.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use overdrive_control_plane::api::{AllocStateWire, AllocStatusResponse, SubmitWorkloadRequest};
use overdrive_control_plane::dns_responder::GuestDnsFactory;
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server_with_obs_and_driver};
use overdrive_core::aggregate::{DriverInput, JobSpecInput, ResourcesInput, VmInput};
use overdrive_core::api::submit::SubmitSpecInput;
use overdrive_core::guest_network::GuestNetworkExecWiring;
use overdrive_core::id::NodeId;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::{SimGuestDnsFactory, SimMtlsIntercept};
use overdrive_worker::mtls_intercept::{
    InterceptElementKey, InterceptElementOperation, InterceptError, InterceptPostcondition,
    InterceptSet, NetlinkError, Result as InterceptResult,
};
use overdrive_worker::mtls_intercept_port::{
    InterceptGuard, InterceptListener, InterceptMembers, InterceptState, MtlsIntercept,
};
use overdrive_worker::mtls_intercept_worker::MtlsInterceptStopError;
use parking_lot::Mutex;
use reqwest::Version;
use std::io::{Read as _, Write as _};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::workload_lifecycle::wait::advance_and_settle;

/// Build a reqwest client that trusts the CA whose PEM lives in the
/// trust triple written by `run_server` during boot.
fn client_trusting(ca_pem: &str) -> reqwest::Client {
    let cert = reqwest::Certificate::from_pem(ca_pem.as_bytes()).expect("parse CA certificate PEM");
    reqwest::Client::builder()
        .add_root_certificate(cert)
        .https_only(true)
        .use_rustls_tls()
        .build()
        .expect("build reqwest client")
}

/// Read the CA PEM out of the ADR-0019 TOML trust triple that
/// `run_server` wrote to `<operator_config_dir>/.overdrive/config`.
fn read_ca_from_trust_triple(operator_config_dir: &std::path::Path) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    let config_path = operator_config_dir.join(".overdrive").join("config");
    let text = std::fs::read_to_string(&config_path)
        .expect(&format!("read trust triple at {}", config_path.display()));

    // ADR-0019 canonical TOML shape: `current-context = "local"` +
    // `[[contexts]]` array-of-tables, each entry carrying `name`,
    // `endpoint`, and the base64-PEM trust triple.
    let doc: toml::Value = toml::from_str(&text).expect("parse trust triple TOML");
    let ca_b64 = doc
        .get("contexts")
        .and_then(toml::Value::as_array)
        .and_then(|arr| {
            arr.iter().find(|c| c.get("name").and_then(toml::Value::as_str) == Some("local"))
        })
        .and_then(|c| c.get("ca"))
        .and_then(toml::Value::as_str)
        .expect("[[contexts]] with name=\"local\" must carry a ca field");
    let ca_bytes = BASE64.decode(ca_b64).expect("base64 decode ca");
    String::from_utf8(ca_bytes).expect("ca PEM is UTF-8")
}

/// Spawn a server on an ephemeral port, return handle + bound-addr +
/// tempdir (kept alive) + CA pem.
///
/// `data_dir` and `operator_config_dir` are SEPARATE subdirectories of
/// the tempdir per `fix-cli-cannot-reach-control-plane` Step 01-02:
/// `data_dir` is the redb + libSQL storage root (ADR-0013 §5);
/// `operator_config_dir` is the trust-triple write target
/// (whitepaper §8, ADR-0019). Decoupling them in tests prevents the
/// overload that hid the production failure (RCA §WHY 4C).
async fn spawn_server() -> (ServerHandle, SocketAddr, TempDir, String) {
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let operator_config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&operator_config_dir).expect("create operator config dir");
    let config = super::node_adapters::test_config(data_dir, operator_config_dir.clone());
    let handle: ServerHandle = super::node_adapters::boot(config).await.expect("run_server");
    let bound: SocketAddr = handle.local_addr().await.expect("bound addr");
    let ca_pem: String = read_ca_from_trust_triple(&operator_config_dir);
    (handle, bound, tmp, ca_pem)
}

// -------------------------------------------------------------------
// S-ND295-54 — graceful shutdown returns the worker's typed teardown failure
// -------------------------------------------------------------------

/// The `InterceptError` the armed removal fault returns for `source_addr`.
fn injected_removal_error(source_addr: Ipv4Addr) -> InterceptError {
    InterceptError::NftElementUpdateFailed {
        set: InterceptSet::ManagedGuestIps,
        operation: InterceptElementOperation::Delete,
        key: InterceptElementKey::Address(source_addr),
        source: NetlinkError::nft(
            "shared-element-remove",
            std::io::Error::from_raw_os_error(libc::EBUSY),
        ),
    }
}

/// The listener type `MtlsIntercept::bind_transparent` returns. The DELIVER
/// step that carries B-7 (05-01 at the latest) changes it to
/// `Arc<dyn InterceptListener>` (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the pinned `bind_transparent` signature)); the delegation below is
/// unchanged by that step.
type BoundListener = Arc<dyn InterceptListener>;

/// Test-local `MtlsIntercept` (the required `ServerConfig.mtls_intercept`)
/// delegating to an inner `SimMtlsIntercept`; `remove_allocation_elements`
/// fails with [`injected_removal_error`] while armed and records each source.
struct RemovalFaultIntercept {
    sim: SimMtlsIntercept,
    armed: AtomicBool,
    removals: Mutex<Vec<Ipv4Addr>>,
}

impl MtlsIntercept for RemovalFaultIntercept {
    fn bind_transparent(&self, addr: SocketAddrV4) -> InterceptResult<BoundListener> {
        self.sim.bind_transparent(addr)
    }

    fn converge_shared(
        &self,
        prior: Option<&InterceptPostcondition>,
        leg_f: SocketAddrV4,
        leg_c: SocketAddrV4,
    ) -> InterceptResult<Box<dyn InterceptGuard>> {
        self.sim.converge_shared(prior, leg_f, leg_c)
    }

    fn observe_shared(&self) -> InterceptResult<Option<InterceptPostcondition>> {
        self.sim.observe_shared()
    }

    fn install_inbound(
        &self,
        virt: SocketAddrV4,
        agent_leg_c_port: u16,
    ) -> InterceptResult<Box<dyn InterceptGuard>> {
        self.sim.install_inbound(virt, agent_leg_c_port)
    }

    fn observe_shared_state(&self) -> InterceptResult<Option<InterceptState>> {
        self.sim.observe_shared_state()
    }

    fn converge_allocation_elements(
        &self,
        expected: &InterceptMembers,
    ) -> InterceptResult<Option<InterceptState>> {
        self.sim.converge_allocation_elements(expected)
    }

    fn remove_allocation_elements(
        &self,
        source_addr: Ipv4Addr,
        destinations: &[SocketAddrV4],
    ) -> InterceptResult<InterceptState> {
        self.removals.lock().push(source_addr);
        if self.armed.load(Ordering::SeqCst) {
            return Err(injected_removal_error(source_addr));
        }
        self.sim.remove_allocation_elements(source_addr, destinations)
    }
}

/// Whether `error`'s source chain carries an `io::Error` with `errno`.
fn chain_carries_errno(error: &(dyn std::error::Error + 'static), errno: i32) -> bool {
    let mut current = Some(error);
    while let Some(link) = current {
        if link.downcast_ref::<std::io::Error>().and_then(std::io::Error::raw_os_error)
            == Some(errno)
        {
            return true;
        }
        current = link.source();
    }
    false
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-54 — Protection removal is convergent and its failures are typed
/// CONTRACT_SHAPE: bounded-change.
///
/// Graceful server shutdown returns the worker's typed teardown failure with
/// no retry capability: a live allocation whose protection members cannot be
/// removed during the owner shutdown surfaces as exactly one
/// `MtlsInterceptStopError::ElementRemoval { alloc_id, source }` naming that
/// allocation, whose `&*source` is the `InterceptError` the required intercept
/// port returned. The fault enters through `ServerConfig.mtls_intercept`; the
/// server is composed by `run_server_with_obs_and_driver`, and the workload is
/// deployed through the public API.
#[allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line; \
              the body is one boot-deploy-fault-shutdown narrative"
)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn graceful_shutdown_propagates_worker_failure_without_a_retry_capability() {
    const TICK: Duration = Duration::from_millis(100);

    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let operator_config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&operator_config_dir).expect("create operator config dir");
    let clock = Arc::new(SimClock::new());
    let clock_port: Arc<dyn Clock> = clock.clone();
    let intercept = Arc::new(RemovalFaultIntercept {
        sim: SimMtlsIntercept::new(),
        armed: AtomicBool::new(false),
        removals: Mutex::new(Vec::new()),
    });
    let intercept_port: Arc<dyn MtlsIntercept> = intercept.clone();
    let guest_dns: Arc<dyn GuestDnsFactory> = Arc::new(SimGuestDnsFactory::default());
    let config = ServerConfig {
        tick_cadence: TICK,
        ..super::node_adapters::test_config(data_dir, operator_config_dir.clone())
    };
    let mut boot = super::node_adapters::boot_adapters(&config);
    boot.clock = Arc::clone(&clock_port);
    boot.mtls.intercept = intercept_port;
    boot.guest_dns = guest_dns;
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    let driver = Arc::new(SimDriver::with_clock(DriverType::Vm, Arc::clone(&clock_port)));
    let driver_port: Arc<dyn Driver> = driver.clone();
    let handle = run_server_with_obs_and_driver(
        config,
        overdrive_control_plane::BootAdapters { observation: obs, ..boot },
        driver_port,
        Arc::new(SimSharedGuestNetworkOwner::default()),
        GuestNetworkExecWiring::new(Arc::clone(&clock_port)),
    )
    .await
    .expect("production boot");
    let bound = handle.local_addr().await.expect("bound addr");
    let client = client_trusting(&read_ca_from_trust_triple(&operator_config_dir));
    let base = format!("https://localhost:{}", bound.port());

    // GIVEN one live allocation deployed through the public API.
    let spec = SubmitSpecInput::Job(JobSpecInput {
        id: "shutdown-job".to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 134_217_728 },
        driver: DriverInput::Vm(VmInput {
            command: "/bin/sleep".to_owned(),
            args: vec!["3600".to_owned()],
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
    });
    let submitted = client
        .post(format!("{base}/v1/workloads"))
        .json(&SubmitWorkloadRequest { spec })
        .send()
        .await
        .expect("POST /v1/workloads");
    assert!(submitted.status().is_success(), "submit: {submitted:?}");
    let mut deployed = None;
    for _ in 0..300 {
        let listing: AllocStatusResponse = client
            .get(format!("{base}/v1/allocs?job=shutdown-job"))
            .send()
            .await
            .expect("GET /v1/allocs")
            .json()
            .await
            .expect("AllocStatusResponse body");
        if let [row] = listing.rows.as_slice()
            && row.state == AllocStateWire::Running
        {
            deployed = Some(row.alloc_id.clone());
            break;
        }
        advance_and_settle(&clock, TICK).await;
    }
    let deployed = deployed.expect("the deployed allocation reaches Running");
    let guest_address = driver
        .started_specs()
        .into_iter()
        .find(|spec| spec.alloc.as_str() == deployed)
        .and_then(|spec| spec.transport)
        .map(|transport| transport.workload_addr())
        .expect("the deployed allocation received its guest-network assignment");

    // WHEN its protection members cannot be removed and the server shuts down.
    intercept.armed.store(true, Ordering::SeqCst);
    let mut shutdown = std::pin::pin!(handle.shutdown(Duration::from_secs(2)));
    let result = loop {
        tokio::select! {
            biased;
            result = &mut shutdown => break result,
            () = tokio::time::sleep(Duration::from_millis(5)) => clock.tick(TICK),
        }
    };

    // THEN graceful shutdown returns exactly the worker's typed failure.
    let failure = result.expect_err("typed worker teardown failure reaches the server caller");
    let failures =
        &failure.teardown_failure().expect("fixture induces mTLS teardown failure").failures;
    assert_eq!(failures.len(), 1, "exactly one allocation-scoped failure: {failures:?}");
    let MtlsInterceptStopError::ElementRemoval { alloc_id, source } = &failures[0] else {
        panic!("expected ElementRemoval, got {:?}", failures[0]);
    };
    assert_eq!(alloc_id.as_str(), deployed, "the failure names the deployed allocation");
    let cause: &InterceptError = source;
    assert!(
        matches!(
            cause,
            InterceptError::NftElementUpdateFailed {
                set: InterceptSet::ManagedGuestIps,
                operation: InterceptElementOperation::Delete,
                key: InterceptElementKey::Address(address),
                ..
            } if *address == guest_address
        ),
        "`&*source` is the InterceptError the intercept port returned: {cause:?}"
    );
    assert_eq!(cause.to_string(), injected_removal_error(guest_address).to_string());
    assert!(chain_carries_errno(cause, libc::EBUSY), "the injected cause is retained: {cause:?}");
    assert!(
        intercept.removals.lock().contains(&guest_address),
        "the owner shutdown asked the port to remove the allocation's members"
    );
}

// -------------------------------------------------------------------
// AC (a) — ephemeral-port bind reported back to the caller
// -------------------------------------------------------------------

#[tokio::test]
async fn run_server_binds_on_ephemeral_port_and_reports_bound_address() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;

    assert!(bound.port() > 0, "expected a non-zero ephemeral port, got {bound}");
    assert_eq!(bound.ip().to_string(), "127.0.0.1", "expected loopback bind, got {}", bound.ip());

    // Prove the server is actually reachable on the reported port
    // before shutdown. This pins the local_addr() return value to the
    // true bound address — a mutation that returned `None` or a
    // fixed `SocketAddr::default()` would fail here.
    let client = client_trusting(&ca_pem);
    let url = format!("https://localhost:{}/v1/cluster/info", bound.port());
    let resp = client.get(&url).send().await.expect("reachable on reported port");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    handle.shutdown(Duration::from_secs(2)).await.expect("clean server shutdown");

    // After graceful shutdown, the listener must be closed — a fresh
    // TCP connect to the same port must fail. This kills the
    // `ServerHandle::shutdown -> ()` mutation: if shutdown did
    // nothing, the port would still be open.
    let closed_result = tokio::net::TcpStream::connect(("127.0.0.1", bound.port())).await;
    assert!(
        closed_result.is_err(),
        "expected ConnectionRefused after shutdown; got {closed_result:?}",
    );
}

// -------------------------------------------------------------------
// AC (b) — reqwest client with minted CA receives 200 on /v1/cluster/info
// -------------------------------------------------------------------

#[tokio::test]
async fn reqwest_client_with_minted_ca_gets_200_on_v1_cluster_info() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);

    let url = format!("https://localhost:{}/v1/cluster/info", bound.port());
    let resp = client.get(&url).send().await.expect("GET /v1/cluster/info");

    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    handle.shutdown(Duration::from_secs(2)).await.expect("clean server shutdown");
}

// -------------------------------------------------------------------
// AC (c) — ALPN negotiates HTTP/2
// -------------------------------------------------------------------

#[tokio::test]
async fn response_alpn_negotiation_is_http_2() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);

    let url = format!("https://localhost:{}/v1/cluster/info", bound.port());
    let resp = client.get(&url).send().await.expect("GET");

    assert_eq!(
        resp.version(),
        Version::HTTP_2,
        "ALPN must negotiate h2 per ADR-0008; got {:?}",
        resp.version(),
    );

    handle.shutdown(Duration::from_secs(2)).await.expect("clean server shutdown");
}

// -------------------------------------------------------------------
// AC (d) — graceful shutdown drains in-flight request
// -------------------------------------------------------------------

#[tokio::test]
async fn cancellation_token_shutdown_drains_in_flight_request() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = Arc::new(client_trusting(&ca_pem));

    // Prime the connection pool with a warmup request — this performs
    // the TCP + TLS + HTTP/2 handshake so the keep-alive connection is
    // open and ready when we issue the "in-flight" request. Without
    // this, reqwest would open a fresh connection AFTER shutdown has
    // already started, and the listener may have already closed.
    let warmup_url = format!("https://localhost:{}/v1/cluster/info", bound.port());
    let warmup = client.get(&warmup_url).send().await.expect("warmup");
    assert_eq!(warmup.status(), reqwest::StatusCode::OK);

    // Now start an in-flight request on the already-pooled connection.
    let in_flight_url = format!("https://localhost:{}/v1/cluster/info", bound.port());
    let client_c = client.clone();
    let in_flight = tokio::spawn(async move { client_c.get(&in_flight_url).send().await });

    // Give the request a moment to land on the wire before we issue
    // shutdown — without this the shutdown may race ahead of the
    // request reaching the server.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Issue graceful shutdown with a 2-second drain window. The
    // in-flight request must still complete with 200 (or at the very
    // least, the server task must not drop it mid-response).
    handle.shutdown(Duration::from_secs(2)).await.expect("clean server shutdown");

    let result = in_flight.await.expect("join");
    let resp = result.expect("in-flight request completes under graceful shutdown");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::OK,
        "in-flight request dropped during graceful shutdown",
    );
}

// -------------------------------------------------------------------
// AC (e) — every ADR-0008 path returns 200 through stub router
// -------------------------------------------------------------------

#[tokio::test]
async fn all_adr_0008_paths_return_200_on_stub_router() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);

    // Per ADR-0008 §Endpoints — Phase 1 endpoint coverage:
    //   - `POST /v1/workloads` real handler (step 03-01)
    //   - `GET /v1/workloads/:id` real handler (step 03-02)
    //   - `GET /v1/allocs?job=<id>` + `GET /v1/nodes` real observation-read
    //     handlers (step 03-03 + slice 01 step 01-03).
    //   - `GET /v1/cluster/info` real handler returning a
    //     `ClusterStatus` body (step 03-05). Per-field content coverage
    //     lives in `acceptance::runtime_registers_noop_heartbeat` and
    //     serde shape is pinned by `acceptance::api_type_shapes`.
    // Per-endpoint happy-path coverage lives in the dedicated scenario
    // modules; this test only pins that the routes remain mounted.
    //
    // `/v1/allocs` requires `?job=<id>` (S-AS-09 / single-cut greenfield);
    // a query against an unknown job returns HTTP 404 with
    // `body.error == "not_found"`. That shape proves both the route is
    // mounted AND the handler is the real one (a stub returning 200
    // would not produce 404). Per ADR-0025 step 5 (wired by step
    // 01-02 of `fix-orphaned-node-health-writer`), a healthy single-
    // node boot writes ONE `node_health` row, so the GET surfaces a
    // single-element rows array — NOT the legacy `{"rows":[]}` shape.
    // The structural assertion is "the `rows` field is present and is
    // a non-empty JSON array"; the per-row content is pinned by
    // `tests/integration/observation_empty_rows.rs::
    //  get_v1_nodes_returns_boot_time_node_health_row_on_fresh_store`.
    let nodes_url = format!("https://localhost:{}/v1/nodes", bound.port());
    let nodes_resp = client.get(&nodes_url).send().await.expect("GET /v1/nodes");
    assert_eq!(nodes_resp.status(), reqwest::StatusCode::OK, "GET /v1/nodes expected 200");
    let nodes_body = nodes_resp.text().await.expect("body");
    assert!(
        nodes_body.starts_with(r#"{"rows":[{"#),
        "GET /v1/nodes must surface the boot-time node_health row inside \
         a non-empty `rows` array; got {nodes_body:?}",
    );

    let allocs_url = format!("https://localhost:{}/v1/allocs?job=ghost-v0", bound.port());
    let allocs_resp = client.get(&allocs_url).send().await.expect("GET /v1/allocs?job=ghost-v0");
    assert_eq!(
        allocs_resp.status(),
        reqwest::StatusCode::NOT_FOUND,
        "GET /v1/allocs?job=<unknown> must surface 404 from the real alloc_status handler",
    );

    // `GET /v1/cluster/info` is a routing check: the body must
    // deserialise into the `ClusterStatus` shape, proving the real
    // handler is wired. Per-field values are pinned by
    // `acceptance::runtime_registers_noop_heartbeat`.
    let url = format!("https://localhost:{}/v1/cluster/info", bound.port());
    let resp = client.get(&url).send().await.expect("GET /v1/cluster/info");
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "GET /v1/cluster/info expected 200");
    let body = resp.text().await.expect("body");
    serde_json::from_str::<overdrive_control_plane::api::ClusterStatus>(&body).expect(
        "GET /v1/cluster/info body must deserialise as ClusterStatus — route must reach the real handler",
    );

    // POST /v1/workloads now routes through the real `submit_workload` handler
    // (step 03-01). A valid body yields 200 + a `SubmitWorkloadResponse`;
    // a malformed body would yield 422 from axum's `Json` extractor,
    // which is precisely why this assertion uses a canonical payload
    // rather than `{}`. Full happy-path + idempotency + conflict
    // coverage lives in `integration::submit_round_trip` — this
    // assertion only pins that the route remains mounted and reachable.
    let url = format!("https://localhost:{}/v1/workloads", bound.port());
    // ADR-0051: `SubmitSpecInput` is `#[serde(tag = "kind")]` with
    // `deny_unknown_fields` — the per-kind discriminator MUST appear
    // inside `spec`. Without it the request fails JSON deserialisation
    // and returns 400, not 200.
    let body = serde_json::json!({
        "spec": {
            "kind": "job",
            "id": "routing-check",
            "replicas": 1,
            "resources": {
                "cpu_milli": 100,
                "memory_bytes": 67_108_864_u64,
            },
            "vm": {
                "command": "/bin/true",
                "args": [],
                "kernel": "/kernel",
                "rootfs": "/rootfs",
            },
        },
    });
    let resp = client.post(&url).json(&body).send().await.expect("POST /v1/workloads");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    handle.shutdown(Duration::from_secs(2)).await.expect("clean server shutdown");
}

// -------------------------------------------------------------------
// Shutdown ends every connection, including stalled TLS handshakes,
// by the drain deadline (public-ingress-gateway R-69 amendment 7a-7e)
// -------------------------------------------------------------------

/// The server's TLS handshake timeout (`axum-server` default). Shutdown must
/// finish far below it: a handshake still stalled when shutdown begins may
/// not hold `ServerHandle::shutdown` until this expires.
const SERVER_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// A real `rustls` client driven step-wise over a raw TCP stream, so a test
/// decides which handshake flight is sent and when.
struct TlsPeer {
    stream: TcpStream,
    conn: rustls::ClientConnection,
}

/// What a peer saw while waiting for the server to close it.
struct Observation {
    /// The server closed the connection (EOF, reset or other transport error).
    closed: bool,
    /// Decrypted application bytes the server sent (a served response).
    plaintext: Vec<u8>,
}

fn tls_client_config(ca_pem: &str) -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls_pemfile::certs(&mut ca_pem.as_bytes()) {
        roots.add(cert.expect("parse CA certificate")).expect("trust minted CA");
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}

impl TlsPeer {
    async fn connect(addr: SocketAddr, config: &Arc<rustls::ClientConfig>) -> Self {
        let stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr))
            .await
            .expect("connect within 5s")
            .expect("connect to the operator address");
        let name = rustls::pki_types::ServerName::try_from("localhost").expect("server name");
        let conn = rustls::ClientConnection::new(Arc::clone(config), name).expect("client");
        Self { stream, conn }
    }

    /// Send every flight the client has queued.
    async fn flush(&mut self) -> std::io::Result<()> {
        let mut out = Vec::new();
        while self.conn.wants_write() {
            self.conn.write_tls(&mut out)?;
        }
        self.stream.write_all(&out).await
    }

    /// Read one chunk from the server and feed it to the TLS state machine.
    /// `Ok(false)` means the server closed the stream.
    async fn read_chunk(&mut self, within: Duration) -> Result<bool, &'static str> {
        let mut buf = vec![0_u8; 16 * 1024];
        match tokio::time::timeout(within, self.stream.read(&mut buf)).await {
            Err(_) => Err("no bytes from the server in time"),
            Ok(Ok(0) | Err(_)) => Ok(false),
            Ok(Ok(n)) => {
                let mut cursor = &buf[..n];
                while !cursor.is_empty() {
                    match self.conn.read_tls(&mut cursor) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    if self.conn.process_new_packets().is_err() {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }

    /// Complete the handshake in full, sending every client flight.
    async fn complete_handshake(&mut self) {
        self.flush().await.expect("send ClientHello");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while self.conn.is_handshaking() {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(
                self.read_chunk(left).await.expect("server flight in time"),
                "the server closed the connection during a handshake that began before shutdown"
            );
        }
        self.flush().await.expect("send client Finished");
    }

    /// Send the `ClientHello` and read the server's whole answering flight,
    /// then stop: the client's final flight stays queued, unsent.
    async fn handshake_until_final_flight_is_withheld(&mut self) {
        self.flush().await.expect("send ClientHello");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while self.conn.is_handshaking() {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(
                self.read_chunk(left).await.expect("server flight in time"),
                "the server closed the connection before shutdown began"
            );
        }
        assert!(self.conn.wants_write(), "the client's final flight is queued and withheld");
    }

    /// Wait up to `within` for the server to close this connection,
    /// collecting any application bytes it sends first.
    async fn observe_until_closed(&mut self, within: Duration) -> Observation {
        let deadline = tokio::time::Instant::now() + within;
        let mut plaintext = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let Ok(alive) = self.read_chunk(left).await else {
                return Observation { closed: false, plaintext };
            };
            let mut buf = [0_u8; 4096];
            while let Ok(n) = self.conn.reader().read(&mut buf) {
                if n == 0 {
                    break;
                }
                plaintext.extend_from_slice(&buf[..n]);
            }
            if !alive {
                return Observation { closed: true, plaintext };
            }
        }
    }
}

/// True once the operator listener refuses new connections, which is how a
/// peer observes that graceful shutdown has begun.
async fn listener_refuses_connections(addr: SocketAddr, within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), TcpStream::connect(addr)).await {
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused => return true,
            _ => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
    false
}

/// Outcome anchor: OUT-IGW-SHUTDOWN-HANDSHAKE-BOUND
/// R-69 7e — Shutdown ends every connection by the drain deadline
/// CONTRACT_SHAPE: bounded-change.
///
/// Four peers hold the operator listener when shutdown begins: (i) a TCP
/// connection that sent the first bytes of a ClientHello and stopped, (ii) a
/// client holding a completed handshake with no request, (iii) a client that
/// sent a full ClientHello and withholds its final flight, and (iv) a client
/// with a request still in flight past the deadline. With a drain deadline of
/// a few hundred milliseconds, `ServerHandle::shutdown` returns within the
/// deadline plus a fixed margin and far below the server's handshake timeout;
/// (i), (ii) and (iv) observe a closed connection; (iii) completes its
/// handshake only after shutdown began and no request is served on it; and the
/// data directory boots again at once, with no sleep and no retry.
#[allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line; \
              the body is one boot-hold-shutdown-observe-reboot narrative"
)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_ends_every_connection_including_stalled_handshakes_by_the_drain_deadline() {
    const DRAIN_DEADLINE: Duration = Duration::from_millis(400);
    // Fixed margin for the other teardown stages; far below the 10 s handshake timeout.
    const MARGIN: Duration = Duration::from_secs(2);
    const OBSERVE: Duration = Duration::from_secs(3);

    let (handle, bound, tmp, ca_pem) = spawn_server().await;
    let data_dir = tmp.path().join("data");
    let operator_config_dir = tmp.path().join("conf");
    let config = tls_client_config(&ca_pem);

    // GIVEN (i) a stalled partial ClientHello: the first bytes only.
    let mut stalled = {
        let mut hello = TlsPeer::connect(bound, &config).await;
        let mut flight = Vec::new();
        hello.conn.write_tls(&mut flight).expect("ClientHello bytes");
        hello.stream.write_all(&flight[..flight.len().min(24)]).await.expect("partial hello");
        hello.stream
    };
    // (ii) a completed handshake holding the connection with no request.
    let mut held = TlsPeer::connect(bound, &config).await;
    held.complete_handshake().await;
    // (iii) a full ClientHello with the final flight withheld.
    let mut withholding = TlsPeer::connect(bound, &config).await;
    withholding.handshake_until_final_flight_is_withheld().await;
    // (iv) one request in flight: headers and one body byte of a 4096 byte body.
    let mut in_flight = TlsPeer::connect(bound, &config).await;
    in_flight.complete_handshake().await;
    in_flight
        .conn
        .writer()
        .write_all(
            b"POST /v1/workloads HTTP/1.1\r\nHost: localhost\r\n\
              Content-Type: application/json\r\nContent-Length: 4096\r\n\r\n{",
        )
        .expect("queue request head");
    in_flight.flush().await.expect("send request head");
    tokio::time::sleep(Duration::from_millis(200)).await;

    // WHEN shutdown begins with a short drain deadline.
    let began = std::time::Instant::now();
    let shutdown = tokio::spawn(handle.shutdown(DRAIN_DEADLINE));
    assert!(
        listener_refuses_connections(bound, Duration::from_secs(2)).await,
        "graceful shutdown never began: the operator listener still accepts connections"
    );

    // AND the stalled client completes its handshake only after shutdown began.
    let completion = withholding.flush().await;
    let _ = withholding.conn.writer().write_all(
        b"GET /v1/cluster/info HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    let _ = withholding.flush().await;

    // THEN shutdown returns by the deadline plus the margin, far below the handshake timeout.
    let joined = tokio::time::timeout(Duration::from_secs(15), shutdown).await;
    let elapsed = began.elapsed();
    let result = joined.expect("shutdown returns at all within 15s").expect("shutdown task joins");
    result.expect("clean server shutdown");
    assert!(
        elapsed <= DRAIN_DEADLINE + MARGIN,
        "shutdown took {elapsed:?}; it must end every connection by the drain deadline \
         {DRAIN_DEADLINE:?} plus {MARGIN:?}, not wait for a stalled handshake"
    );
    assert!(
        elapsed < SERVER_HANDSHAKE_TIMEOUT / 2,
        "shutdown took {elapsed:?}, near the handshake timeout"
    );

    // AND every held connection observes a closed connection.
    let mut buf = [0_u8; 64];
    let stalled_closed =
        matches!(tokio::time::timeout(OBSERVE, stalled.read(&mut buf)).await, Ok(Ok(0) | Err(_)));
    assert!(stalled_closed, "the stalled partial ClientHello connection is closed by the server");
    let held_seen = held.observe_until_closed(OBSERVE).await;
    assert!(held_seen.closed, "the idle handshaken connection is closed by the server");
    let in_flight_seen = in_flight.observe_until_closed(OBSERVE).await;
    assert!(in_flight_seen.closed, "the request still in flight past the deadline is dropped");
    assert!(
        !String::from_utf8_lossy(&in_flight_seen.plaintext).contains("200 OK"),
        "the unfinished request is not answered as a success"
    );

    // AND the late completion fails: the connection is closed and no request is served on it.
    let late_seen = withholding.observe_until_closed(OBSERVE).await;
    assert!(
        late_seen.closed && late_seen.plaintext.is_empty(),
        "late handshake completion (flush: {completion:?}) must fail with no response served; \
         closed={}, {} plaintext bytes",
        late_seen.closed,
        late_seen.plaintext.len(),
    );

    // AND the data directory boots again at once: no sleep, no retry.
    let reboot_config = super::node_adapters::test_config(data_dir, operator_config_dir);
    let rebooted =
        tokio::time::timeout(Duration::from_secs(30), super::node_adapters::boot(reboot_config))
            .await
            .expect("reboot over the same data directory within 30s")
            .expect("every store handle was released on return from shutdown");
    rebooted.shutdown(Duration::from_secs(2)).await.expect("clean second shutdown");
}
