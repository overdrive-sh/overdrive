//! Tier 3 integration — `HyperHttpProber` against a real in-process
//! tokio-spawned hyper HTTP server inside Lima.
//!
//! Slice 02 (US-02). Per `.claude/rules/testing.md` § "Integration vs
//! unit gating": real-network test belongs in the `integration-tests`
//! slow lane. Per § "Running tests — Lima VM": invocation goes through
//! `cargo xtask lima run -- cargo nextest run -p overdrive-worker
//! --features integration-tests -E 'test(real_http_probe)'`.
//!
//! The server is a hyper `1.x` connection served over a real
//! `tokio::net::TcpListener` bound to `127.0.0.1:0` (kernel-assigned
//! ephemeral port; no race). Each test spawns a single-shot acceptor:
//! one connection, one response with the test's status code, then the
//! acceptor task ends. The probe fires a single GET; no redirect is
//! followed (the 302 case asserts this).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::convert::Infallible;
use std::time::Duration;

use http_body_util::Empty;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Response, StatusCode};
use hyper_util::rt::TokioIo;
use overdrive_core::traits::prober::HttpProber;
use overdrive_core::traits::prober::ProbeOutcome;
use overdrive_worker::probe_runner::HyperHttpProber;
use tokio::net::TcpListener;

/// Bind a real loopback listener and spawn an acceptor that answers a
/// single connection with `status` (and, for redirects, a `Location`
/// header). Returns the bound `http://127.0.0.1:<port>` base URL so
/// the probe can target it.
async fn spawn_single_shot_server(status: StatusCode, location: Option<&'static str>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind loopback listener");
    let addr = listener.local_addr().expect("read local addr");

    tokio::spawn(async move {
        // Accept exactly one connection — the single probe GET.
        let (stream, _peer) = listener.accept().await.expect("accept one connection");
        let io = TokioIo::new(stream);
        let service = service_fn(move |_req| async move {
            let mut builder = Response::builder().status(status);
            if let Some(loc) = location {
                builder = builder.header("location", loc);
            }
            let resp = builder.body(Empty::<Bytes>::new()).expect("build response with empty body");
            Ok::<_, Infallible>(resp)
        });
        // Serve the single connection; ignore the per-connection
        // result (the probe drops the connection after reading the
        // status line, which surfaces as a benign incomplete-message
        // on the server side).
        let _ = http1::Builder::new().serve_connection(io, service).await;
    });

    format!("http://127.0.0.1:{}", addr.port())
}

/// S-SHCP-INT-02-01 (US-02 / K1) — real HTTP server returns 200 OK;
/// `HyperHttpProber` returns Pass.
#[tokio::test]
async fn given_real_http_server_200_when_hyper_http_prober_probes_then_returns_pass() {
    let base = spawn_single_shot_server(StatusCode::OK, None).await;
    let url = format!("{base}/healthz");

    let prober = HyperHttpProber::new();
    let outcome = prober
        .probe(&url, Duration::from_secs(5))
        .await
        .expect("probe call returns Ok against a real 200 server");

    assert!(
        matches!(outcome, ProbeOutcome::Pass),
        "expected Pass against a real HTTP 200 server; got {outcome:?}"
    );
}

/// S-SHCP-INT-02-02 (US-02 / K1) — real HTTP server returns 503;
/// `HyperHttpProber` returns `Fail { reason: "HTTP 503" }`.
#[tokio::test]
async fn given_real_http_server_503_when_hyper_http_prober_probes_then_returns_fail_named() {
    let base = spawn_single_shot_server(StatusCode::SERVICE_UNAVAILABLE, None).await;
    let url = format!("{base}/healthz");

    let prober = HyperHttpProber::new();
    let outcome = prober
        .probe(&url, Duration::from_secs(5))
        .await
        .expect("probe call returns Ok against a real 503 server");

    match outcome {
        ProbeOutcome::Fail { reason } => {
            assert_eq!(reason, "HTTP 503", "expected named 503 reason; got {reason:?}");
        }
        ProbeOutcome::Pass => panic!("expected Fail(HTTP 503); got Pass"),
    }
}

/// S-SHCP-INT-02-03 (US-02 AC / research § 6.1 Pitfall 5) — real HTTP
/// server returns 302 with a `Location` header; `HyperHttpProber`
/// returns Fail (NOT Pass; the redirect is NOT followed).
#[tokio::test]
async fn given_real_http_server_302_when_hyper_http_prober_probes_then_returns_fail_no_follow() {
    let base =
        spawn_single_shot_server(StatusCode::FOUND, Some("http://127.0.0.1:1/elsewhere")).await;
    let url = format!("{base}/healthz");

    let prober = HyperHttpProber::new();
    let outcome = prober
        .probe(&url, Duration::from_secs(5))
        .await
        .expect("probe call returns Ok against a real 302 server");

    match outcome {
        ProbeOutcome::Fail { reason } => {
            assert_eq!(
                reason, "HTTP 302 (redirect not followed)",
                "302 must be Fail naming no-redirect-follow; got {reason:?}"
            );
        }
        ProbeOutcome::Pass => {
            panic!("expected Fail(HTTP 302 ...); got Pass — redirect was followed")
        }
    }
}

/// S-SHCP-02-04 (US-02 / K1, real-transport variant) — a GET against
/// an unbound loopback port surfaces the kernel ECONNREFUSED through
/// the real `hyper-util` client error chain as
/// `Fail { reason: "connection refused" }`. This exercises
/// `client_error_to_reason` (the transport-error mapping the Tier-1
/// queue-driven acceptance test cannot reach — the sim adapter carries
/// the reason verbatim rather than deriving it from a real error).
#[tokio::test]
async fn given_unbound_port_when_hyper_http_prober_probes_then_returns_fail_connection_refused() {
    // Bind to discover a kernel-assigned ephemeral port, then drop the
    // listener so the port is unbound; a connect now refuses.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind loopback listener");
    let addr = listener.local_addr().expect("read local addr");
    drop(listener);
    let url = format!("http://127.0.0.1:{}/healthz", addr.port());

    let prober = HyperHttpProber::new();
    let outcome = prober
        .probe(&url, Duration::from_secs(5))
        .await
        .expect("probe call returns Ok even when kernel refuses");

    match outcome {
        ProbeOutcome::Fail { reason } => {
            assert_eq!(
                reason, "connection refused",
                "expected named ECONNREFUSED reason; got {reason:?}"
            );
        }
        ProbeOutcome::Pass => panic!("expected Fail(connection refused); got Pass"),
    }
}
