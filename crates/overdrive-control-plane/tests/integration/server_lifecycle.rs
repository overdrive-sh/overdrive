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
//! * A `reqwest::Client` trusting the minted CA performs a real TLS 1.3
//!   + HTTP/2 handshake against the server and receives HTTP 200 for
//!   every one of the five ADR-0008 endpoint paths from the stub
//!   router.
//! * ALPN negotiation produces HTTP/2 (not HTTP/1.1).
//! * A `CancellationToken` triggers `graceful_shutdown`; an in-flight
//!   request that began before cancellation still completes with 200.
//!
//! All tests run the server in a Tokio task and invoke reqwest against
//! it — real sockets, real TLS handshake, real HTTP parsing. This is
//! Tier 3 real-network integration per `.claude/rules/testing.md`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::{ServerConfig, ServerHandle, run_server};
use reqwest::Version;
use tempfile::TempDir;

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

/// Read the CA PEM out of the trust-triple YAML that `run_server`
/// wrote to `data_dir/.overdrive/config`.
fn read_ca_from_trust_triple(data_dir: &std::path::Path) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    let config_path = data_dir.join(".overdrive").join("config");
    let yaml = std::fs::read_to_string(&config_path)
        .expect(&format!("read trust triple at {}", config_path.display()));

    let doc: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("parse trust triple YAML");
    let ca_b64 = doc
        .get("contexts")
        .and_then(|c| c.get("local"))
        .and_then(|c| c.get("ca"))
        .and_then(|v| v.as_str())
        .expect("contexts.local.ca field");
    let ca_bytes = BASE64.decode(ca_b64).expect("base64 decode ca");
    String::from_utf8(ca_bytes).expect("ca PEM is UTF-8")
}

/// Spawn a server on an ephemeral port, return handle + bound-addr +
/// tempdir (kept alive) + CA pem.
async fn spawn_server() -> (ServerHandle, SocketAddr, TempDir, String) {
    let tmp = TempDir::new().expect("tempdir");
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("parse bind addr"),
        data_dir: tmp.path().to_path_buf(),
    };
    let handle: ServerHandle = run_server(config).await.expect("run_server");
    let bound: SocketAddr = handle.local_addr().await.expect("bound addr");
    let ca_pem: String = read_ca_from_trust_triple(tmp.path());
    (handle, bound, tmp, ca_pem)
}

// -------------------------------------------------------------------
// AC (a) — ephemeral-port bind reported back to the caller
// -------------------------------------------------------------------

#[tokio::test]
async fn run_server_binds_on_ephemeral_port_and_reports_bound_address() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;

    assert!(bound.port() > 0, "expected a non-zero ephemeral port, got {bound}",);
    assert_eq!(bound.ip().to_string(), "127.0.0.1", "expected loopback bind, got {}", bound.ip(),);

    // Prove the server is actually reachable on the reported port
    // before shutdown. This pins the local_addr() return value to the
    // true bound address — a mutation that returned `None` or a
    // fixed `SocketAddr::default()` would fail here.
    let client = client_trusting(&ca_pem);
    let url = format!("https://localhost:{}/v1/cluster/info", bound.port());
    let resp = client.get(&url).send().await.expect("reachable on reported port");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    handle.shutdown(Duration::from_secs(2)).await;

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

    handle.shutdown(Duration::from_secs(2)).await;
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

    handle.shutdown(Duration::from_secs(2)).await;
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
    handle.shutdown(Duration::from_secs(2)).await;

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
    //   - `POST /v1/jobs` real handler (step 03-01)
    //   - `GET /v1/jobs/:id` real handler (step 03-02)
    //   - `GET /v1/allocs` + `GET /v1/nodes` real observation-read
    //     handlers returning `{"rows":[]}` on a fresh store (step 03-03)
    //   - `GET /v1/cluster/info` real handler returning a
    //     `ClusterStatus` body (step 03-05). Per-field content coverage
    //     lives in `acceptance::runtime_registers_noop_heartbeat` and
    //     serde shape is pinned by `acceptance::api_type_shapes`.
    // Per-endpoint happy-path coverage lives in the dedicated scenario
    // modules; this test only pins that the routes remain mounted.
    let observation_gets = ["/v1/allocs", "/v1/nodes"];
    for path in observation_gets {
        let url = format!("https://localhost:{}{path}", bound.port());
        let resp = client.get(&url).send().await.expect(&format!("GET {path}"));
        assert_eq!(resp.status(), reqwest::StatusCode::OK, "GET {path} expected 200",);
        let body = resp.text().await.expect("body");
        assert_eq!(
            body, r#"{"rows":[]}"#,
            "fresh-store observation read must surface explicit empty rows array"
        );
    }

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

    // POST /v1/jobs now routes through the real `submit_job` handler
    // (step 03-01). A valid body yields 200 + a `SubmitJobResponse`;
    // a malformed body would yield 422 from axum's `Json` extractor,
    // which is precisely why this assertion uses a canonical payload
    // rather than `{}`. Full happy-path + idempotency + conflict
    // coverage lives in `integration::submit_round_trip` — this
    // assertion only pins that the route remains mounted and reachable.
    let url = format!("https://localhost:{}/v1/jobs", bound.port());
    let body = serde_json::json!({
        "spec": {
            "id": "routing-check",
            "replicas": 1,
            "cpu_milli": 100,
            "memory_bytes": 67_108_864_u64,
        },
    });
    let resp = client.post(&url).json(&body).send().await.expect("POST /v1/jobs");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    handle.shutdown(Duration::from_secs(2)).await;
}
