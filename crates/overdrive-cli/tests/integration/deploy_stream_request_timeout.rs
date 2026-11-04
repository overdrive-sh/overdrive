//! Feature `service-allocation-readiness-and-ending`, decision D10 item 7, obligation
//! S-36 (request-timeout probe) — the total request timeout of the streaming submit
//! lane is the server's stream deadline plus a 30 s envelope.
//!
//! A TLS server answers the submit with `200 application/x-ndjson`, yields its first
//! line and then holds the stream open for ever. The client has the response headers
//! and the first line in real time; the probe then pauses the clock, so the stalled body
//! read ends at the client's own total timeout without a wall-clock wait, and the
//! virtual time measured from the request is the timeout the client armed:
//! `deadline + 30 s` for the Service lane (the server's `progress_deadline`) and
//! `DEFAULT_STREAMING_CAP + 30 s` (120 s) for the Job lane. The CLI never spawns a
//! subprocess here: the client method is called directly
//! (`crates/overdrive-cli/CLAUDE.md`).
//!
//! CONTRACT_SHAPE: bounded-change. Universe: the time the stalled response body takes
//! to end, and the typed error it ends with.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::doc_markdown)]

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Response, header};
use axum::routing::post;
use bytes::Bytes;
use futures::StreamExt as _;
use overdrive_cli::http_client::ApiClient;
use overdrive_control_plane::DEFAULT_STREAMING_CAP;
use overdrive_control_plane::api::SubmitWorkloadRequest;
use overdrive_control_plane::tls_bootstrap::{
    load_server_tls_config, mint_ephemeral_ca, write_trust_triple,
};
use overdrive_core::aggregate::{DriverInput, ResourcesInput};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use tempfile::TempDir;

/// A control plane that answers the submit, yields one NDJSON line and then stalls.
fn stalled_stream_endpoint() -> (ApiClient, TempDir) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let material = mint_ephemeral_ca().expect("an ephemeral CA");
    let server_config = load_server_tls_config(&material).expect("the server TLS config");
    let app = Router::new().route(
        "/v1/workloads",
        post(|| async {
            let stalled = futures::stream::once(async {
                Ok::<_, std::io::Error>(Bytes::from_static(b"{\"kind\":\"accepted\"}\n"))
            })
            .chain(futures::stream::pending());
            Response::builder()
                .header(header::CONTENT_TYPE, "application/x-ndjson")
                .body(Body::from_stream(stalled))
                .expect("a response")
        }),
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("non-blocking");
    let port = listener.local_addr().expect("addr").port();
    let rustls = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config));
    tokio::spawn(async move {
        let _ = axum_server::from_tcp_rustls(listener, rustls).serve(app.into_make_service()).await;
    });
    let config_dir = TempDir::new().expect("tempdir");
    write_trust_triple(config_dir.path(), &format!("https://127.0.0.1:{port}"), &material)
        .expect("trust triple");
    let client = ApiClient::from_config(&config_dir.path().join(".overdrive").join("config"))
        .expect("a client over the stalled endpoint");
    (client, config_dir)
}

fn service_request() -> SubmitWorkloadRequest {
    SubmitWorkloadRequest {
        spec: SubmitSpecInput::Service(ServiceSpecInput {
            id: "web".to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            driver: DriverInput::Vm(overdrive_core::aggregate::VmInput {
                command: "/bin/sleep".to_owned(),
                args: vec!["3600".to_owned()],
                kernel: "/kernel".to_owned(),
                rootfs: "/rootfs".to_owned(),
            }),
            listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
            startup_probes: vec![],
            readiness_probes: vec![],
            liveness_probes: vec![],
        }),
    }
}

/// How long, from the request, the stalled response body takes to end in an error when
/// the server announces `deadline`.
async fn time_until_the_stalled_body_ends(client: &ApiClient, deadline: Duration) -> Duration {
    let requested = tokio::time::Instant::now();
    let response = client
        .submit_workload_streaming(service_request(), deadline)
        .await
        .expect("the response headers arrive");
    let mut body = response.bytes_stream();
    let first = body.next().await.expect("the first line").expect("bytes");
    assert!(first.starts_with(b"{\"kind\":\"accepted\""), "the first line arrives");
    // Everything real has happened: from here only the client's own total timeout is
    // pending, so the clock may run ahead to it.
    tokio::time::pause();
    let outcome = body.next().await;
    assert!(matches!(outcome, Some(Err(_))), "the stalled body ends in an error; got {outcome:?}");
    requested.elapsed()
}

/// S-36 — a Service whose server deadline is 150 s is not cut at the old fixed 120 s:
/// the client waits `deadline + 30 s` = 180 s, so the typed terminal the server was about
/// to emit is not hidden.
#[tokio::test]
#[ignore = "pending DELIVER: D10 total request timeout is the server deadline plus 30 s"]
async fn the_service_lane_waits_the_server_deadline_plus_thirty_seconds() {
    let (client, _config) = stalled_stream_endpoint();

    let waited = time_until_the_stalled_body_ends(&client, Duration::from_secs(150)).await;

    assert!(
        waited >= Duration::from_secs(180) && waited < Duration::from_secs(182),
        "the request timeout is deadline + 30 s = 180 s; waited {waited:?}",
    );
}

/// S-36 — the Job lane passes `DEFAULT_STREAMING_CAP` and keeps its 120 s envelope,
/// byte-identical to today.
#[tokio::test]
async fn the_job_lane_keeps_the_one_hundred_twenty_second_envelope() {
    let (client, _config) = stalled_stream_endpoint();

    let waited = time_until_the_stalled_body_ends(&client, DEFAULT_STREAMING_CAP).await;

    assert!(
        waited >= Duration::from_secs(120) && waited < Duration::from_secs(122),
        "DEFAULT_STREAMING_CAP + 30 s = 120 s; waited {waited:?}",
    );
}
