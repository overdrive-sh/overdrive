//! `HyperHttpProber` — production binding of `HttpProber` over
//! `hyper-util::client::legacy::Client` + `tokio::time::timeout`.
//!
//! Per ADR-0054 §4 + DDD-20: `hyper-util` 1.x is the workspace HTTP
//! client. GET only per US-02; the probe does NOT follow redirects —
//! a 3xx response is a `Fail`, never followed (US-02 AC; research
//! § 6.1 Pitfall 5). The classification table:
//!
//! | Response / outcome | `ProbeOutcome` |
//! |---|---|
//! | HTTP 2xx | `Pass` |
//! | HTTP 3xx | `Fail { reason: "HTTP <code> (redirect not followed)" }` |
//! | HTTP 4xx / 5xx | `Fail { reason: "HTTP <code>" }` |
//! | Connection refused | `Fail { reason: "connection refused" }` |
//! | Timeout (per-request `tokio::time::timeout` elapsed) | `Fail { reason: "timeout after <N>" }` |
//!
//! These `reason` strings are the operator-facing contract per
//! [`ProbeOutcome::Fail`]'s docstring — renaming them is a wire-shape
//! change.

#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::too_long_first_doc_paragraph,
    clippy::module_name_repetitions,
    clippy::missing_errors_doc,
    reason = "shared docstring style for the ProbeRunner subsystem"
)]

use std::time::Duration;

use async_trait::async_trait;
use http_body_util::Empty;
use hyper::Request;
use hyper::body::Bytes;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use overdrive_core::traits::prober::{HttpProber, ProbeFailure, ProbeOutcome};

/// Production `HttpProber` over `hyper-util`.
pub struct HyperHttpProber;

impl HyperHttpProber {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for HyperHttpProber {
    fn default() -> Self {
        Self::new()
    }
}

/// Classify an HTTP status code into a [`ProbeOutcome`] per the US-02
/// AC table. Pure function — the SUT for the
/// `HttpProberStatusCodeClassification` proptest (universe `0..=999`).
///
/// - `200..=299` → [`ProbeOutcome::Pass`].
/// - `300..=399` → [`ProbeOutcome::Fail`] with reason
///   `"HTTP <code> (redirect not followed)"`. The probe does NOT
///   follow redirects (research § 6.1 Pitfall 5) — the reason names
///   that the redirect was not followed.
/// - everything else (including `400..=599` and any non-standard
///   code the wire carries) → [`ProbeOutcome::Fail`] with reason
///   `"HTTP <code>"`.
#[must_use]
pub fn classify_http_status(code: u16) -> ProbeOutcome {
    match code {
        200..=299 => ProbeOutcome::Pass,
        300..=399 => ProbeOutcome::Fail { reason: format!("HTTP {code} (redirect not followed)") },
        _ => ProbeOutcome::Fail { reason: format!("HTTP {code}") },
    }
}

/// Format a `Duration` for operator-facing reason strings — mirrors
/// the TCP prober shape (`"timeout after 5s"`, `"timeout after
/// 250ms"`).
fn format_duration(d: Duration) -> String {
    let total_ms = d.as_millis();
    if total_ms >= 1000 && total_ms % 1000 == 0 {
        format!("{}s", d.as_secs())
    } else if total_ms >= 1000 {
        format!("{:.1}s", d.as_secs_f64())
    } else {
        format!("{total_ms}ms")
    }
}

#[async_trait]
impl HttpProber for HyperHttpProber {
    async fn probe(&self, url: &str, timeout: Duration) -> Result<ProbeOutcome, ProbeFailure> {
        // Input validation mirrors the sim adapter — per
        // `nw-tdd-methodology` § "Test Doubles Must Validate Inputs".
        if url.is_empty() {
            return Err(ProbeFailure::InvalidTarget {
                reason: "http probe url must be non-empty".to_owned(),
            });
        }
        if !url.starts_with("http://") {
            return Err(ProbeFailure::InvalidTarget {
                reason: format!("http probe url must start with `http://`; got {url:?}"),
            });
        }

        // GET only per US-02. Empty request body. The client is
        // constructed per-probe — probes are infrequent (interval ≥ 2s)
        // so connection-pool reuse across probes buys nothing, and a
        // fresh client guarantees no stale-connection masking.
        let client: Client<_, Empty<Bytes>> = Client::builder(TokioExecutor::new()).build_http();
        let request = match Request::builder().method("GET").uri(url).body(Empty::<Bytes>::new()) {
            Ok(req) => req,
            Err(err) => {
                return Err(ProbeFailure::InvalidTarget {
                    reason: format!("http probe url failed to build request: {err}"),
                });
            }
        };

        match tokio::time::timeout(timeout, client.request(request)).await {
            // Per-request timeout elapsed before the client returned.
            Err(_elapsed) => Ok(ProbeOutcome::Fail {
                reason: format!("timeout after {}", format_duration(timeout)),
            }),
            // The request completed within timeout — classify on the
            // status code. The probe does NOT follow redirects; the
            // 3xx status is classified as Fail in place.
            Ok(Ok(response)) => Ok(classify_http_status(response.status().as_u16())),
            // Transport-layer error (connection refused, reset, DNS).
            Ok(Err(err)) => Ok(ProbeOutcome::Fail { reason: client_error_to_reason(&err) }),
        }
    }
}

/// Translate a `hyper-util` legacy-client error into the
/// operator-renderable `reason` string. Connection-refused is the
/// load-bearing case (US-02 AC); everything else surfaces a stable
/// transport-shaped string so the renderer passes it through.
fn client_error_to_reason(err: &hyper_util::client::legacy::Error) -> String {
    // The legacy client wraps the underlying `std::io::Error` in its
    // source chain. Walk the chain for an `io::Error` so a kernel-side
    // ECONNREFUSED renders as the named `"connection refused"` reason
    // rather than the verbose hyper wrapper text.
    let mut source: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(err);
    while let Some(cause) = source {
        if let Some(io_err) = cause.downcast_ref::<std::io::Error>() {
            return io_error_to_reason(io_err);
        }
        source = cause.source();
    }
    format!("http request failed: {err}")
}

/// Map an `std::io::Error` surfaced through the client error chain
/// into a stable operator-facing reason string.
fn io_error_to_reason(err: &std::io::Error) -> String {
    use std::io::ErrorKind;
    match err.kind() {
        ErrorKind::ConnectionRefused => "connection refused".to_owned(),
        ErrorKind::ConnectionReset => "connection reset".to_owned(),
        other => format!("connect failed: {other:?}: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_renders_whole_seconds_and_millis() {
        assert_eq!(format_duration(Duration::from_secs(5)), "5s");
        assert_eq!(format_duration(Duration::from_millis(250)), "250ms");
        assert_eq!(format_duration(Duration::from_millis(1500)), "1.5s");
    }

    #[test]
    fn classify_boundaries_are_inclusive() {
        // Pins the exact band edges against off-by-one mutants on the
        // range guards in `classify_http_status`.
        assert_eq!(classify_http_status(199), ProbeOutcome::Fail { reason: "HTTP 199".to_owned() });
        assert_eq!(classify_http_status(200), ProbeOutcome::Pass);
        assert_eq!(classify_http_status(299), ProbeOutcome::Pass);
        assert_eq!(
            classify_http_status(300),
            ProbeOutcome::Fail { reason: "HTTP 300 (redirect not followed)".to_owned() }
        );
        assert_eq!(
            classify_http_status(399),
            ProbeOutcome::Fail { reason: "HTTP 399 (redirect not followed)".to_owned() }
        );
        assert_eq!(classify_http_status(400), ProbeOutcome::Fail { reason: "HTTP 400".to_owned() });
    }

    #[test]
    fn io_connection_refused_maps_to_named_reason() {
        let err = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
        assert_eq!(io_error_to_reason(&err), "connection refused");
    }

    #[test]
    fn io_connection_reset_maps_to_named_reason() {
        let err = std::io::Error::from(std::io::ErrorKind::ConnectionReset);
        assert_eq!(io_error_to_reason(&err), "connection reset");
    }

    #[test]
    fn io_other_error_falls_through_to_catch_all() {
        let err = std::io::Error::other("some other failure");
        let rendered = io_error_to_reason(&err);
        assert!(
            rendered.starts_with("connect failed: "),
            "expected catch-all prefix, got {rendered:?}"
        );
    }
}
