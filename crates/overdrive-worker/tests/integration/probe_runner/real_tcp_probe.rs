//! Tier 3 integration — `TokioTcpProber` against real loopback
//! sockets inside Lima.
//!
//! Per `.claude/rules/testing.md` § "Integration vs unit gating":
//! real-network test belongs in the `integration-tests` slow lane.
//! Per `.claude/rules/testing.md` § "Running tests — Lima VM":
//! invocation goes through `cargo xtask lima run -- cargo nextest
//! run -p overdrive-worker --features integration-tests -E
//! 'test(real_tcp_probe)'`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::time::Duration;

use overdrive_core::traits::prober::{ProbeOutcome, TcpProber};
use overdrive_worker::probe_runner::TokioTcpProber;
use tokio::net::TcpListener;

/// S-SHCP-INT-01-01 (US-01 WS / K1) — happy path: bind a real
/// loopback listener on `127.0.0.1:0` (kernel-assigned port; no
/// race per ADR-0054 §7), probe it, assert Pass.
#[tokio::test]
async fn given_real_loopback_listener_when_tokio_tcp_prober_probes_then_returns_pass() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind loopback listener");
    let addr = listener.local_addr().expect("read local addr");

    let prober = TokioTcpProber::new();
    let outcome = prober
        .probe(&addr.ip().to_string(), addr.port(), Duration::from_secs(5))
        .await
        .expect("loopback probe call returns Ok");

    // Hold listener open until after the probe completes — the
    // handshake target must remain accepting throughout the probe.
    drop(listener);
    assert!(
        matches!(outcome, ProbeOutcome::Pass),
        "expected Pass against an accepting loopback listener; got {outcome:?}"
    );
}

/// S-SHCP-INT-01-02 (US-01 WS / K1 sad path) — connection refused
/// against an unbound port surfaces as
/// `Fail { reason: "connection refused" }`.
#[tokio::test]
async fn given_unbound_port_when_tokio_tcp_prober_probes_then_returns_fail_connection_refused() {
    // Bind a listener to discover an ephemeral port the kernel has
    // ALREADY assigned, then drop the listener immediately. The port
    // is now unbound; the kernel returns ECONNREFUSED on connect.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind loopback listener");
    let addr = listener.local_addr().expect("read local addr");
    drop(listener);

    let prober = TokioTcpProber::new();
    let outcome = prober
        .probe(&addr.ip().to_string(), addr.port(), Duration::from_secs(5))
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

/// S-SHCP-INT-01-03 (US-01 WS / K1 sad path) — bound-but-not-
/// accepting listener with a short timeout surfaces as
/// `Fail { reason: "timeout after <duration>" }`.
///
/// Lima loopback is reliable enough that `127.0.0.1:0` with the
/// listener still bound does not reliably timeout (the kernel
/// accepts the SYN into the backlog even without a userspace
/// `accept()`). The deterministic timeout shape is a non-listening
/// non-loopback address that drops SYN packets — TEST-NET-1
/// (192.0.2.0/24, RFC 5737) is reserved for documentation and
/// never routable; a connect attempt blocks until the timeout
/// elapses.
#[tokio::test]
async fn given_blackhole_address_when_tokio_tcp_prober_probes_then_returns_fail_timeout() {
    let prober = TokioTcpProber::new();
    // RFC 5737 TEST-NET-1 — reserved for documentation, never
    // routable; connect attempts block until timeout.
    let outcome = prober
        .probe("192.0.2.1", 80, Duration::from_millis(250))
        .await
        .expect("probe call returns Ok even on timeout");

    match outcome {
        ProbeOutcome::Fail { reason } => {
            assert!(
                reason.starts_with("timeout after "),
                "expected timeout-shaped reason; got {reason:?}"
            );
        }
        ProbeOutcome::Pass => panic!("expected Fail(timeout); got Pass"),
    }
}
