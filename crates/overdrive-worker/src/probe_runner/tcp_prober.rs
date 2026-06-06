//! `TokioTcpProber` — production binding of `TcpProber` over
//! `tokio::net::TcpStream` + `tokio::time::timeout`.
//!
//! Per ADR-0054 §4: real socket per attempt; immediate drop on
//! handshake success (no data sent or expected).
//!
//! Phase 1 maps every kernel-side connect failure to a stable
//! operator-renderable string in `ProbeOutcome::Fail { reason }`:
//!
//! | Error / outcome | `reason` string |
//! |---|---|
//! | Handshake completed within `timeout` | _(returns `Pass`)_ |
//! | `tokio::time::timeout` elapsed | `"timeout after <duration>"` |
//! | `io::ErrorKind::ConnectionRefused` | `"connection refused"` |
//! | `io::ErrorKind::TimedOut` (kernel ETIMEDOUT) | `"timeout after <duration>"` |
//! | DNS resolution failure | `"dns: <error>"` |
//! | other `io::Error` | `"connect failed: <kind>: <message>"` |
//!
//! These strings are the operator-facing contract per
//! `ProbeOutcome::Fail`'s docstring — renaming them is a wire-shape
//! change.

use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::traits::prober::{ProbeFailure, ProbeOutcome, TcpProber};
use tokio::net::TcpStream;

/// Production `TcpProber` over `tokio::net`.
pub struct TokioTcpProber;

impl TokioTcpProber {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for TokioTcpProber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TcpProber for TokioTcpProber {
    async fn probe(
        &self,
        host: &str,
        port: u16,
        timeout: Duration,
    ) -> Result<ProbeOutcome, ProbeFailure> {
        // Input validation mirrors the sim adapter — per
        // `nw-tdd-methodology` § "Test Doubles Must Validate Inputs".
        // The trait docstring documents the parse-time precondition;
        // we enforce it again at the boundary as defense in depth.
        if host.is_empty() {
            return Err(ProbeFailure::InvalidTarget {
                reason: "tcp probe host must be non-empty".to_owned(),
            });
        }
        if port == 0 {
            return Err(ProbeFailure::InvalidTarget {
                reason: "tcp probe port must be in 1..=65535".to_owned(),
            });
        }

        let target = format!("{host}:{port}");
        match tokio::time::timeout(timeout, TcpStream::connect(&target)).await {
            // Timeout elapsed before the kernel returned.
            Err(_elapsed) => Ok(ProbeOutcome::Fail {
                reason: format!("timeout after {}", format_duration(timeout)),
            }),
            // Connect returned within `timeout`.
            Ok(Ok(_stream)) => {
                // Drop the stream immediately — no data is sent or
                // expected. The handshake completing IS the success
                // signal per ADR-0054 §4.
                Ok(ProbeOutcome::Pass)
            }
            Ok(Err(err)) => {
                Ok(ProbeOutcome::Fail { reason: connect_error_to_reason(&err, timeout) })
            }
        }
    }
}

/// Translate a `tokio::net::TcpStream::connect` error into the
/// operator-renderable `reason` string per the table in the module
/// docstring.
fn connect_error_to_reason(err: &std::io::Error, timeout: Duration) -> String {
    use std::io::ErrorKind;
    match err.kind() {
        ErrorKind::ConnectionRefused => "connection refused".to_owned(),
        ErrorKind::TimedOut => format!("timeout after {}", format_duration(timeout)),
        // tokio's DNS resolution surfaces through std::io::Error;
        // distinguish by the inner message kind. The kind `Other`
        // with a `failed to lookup address` body is the canonical
        // DNS-resolution-failure shape.
        ErrorKind::Other if err.to_string().contains("failed to lookup address") => {
            format!("dns: {err}")
        }
        other => format!("connect failed: {other:?}: {err}"),
    }
}

/// Format a `Duration` for operator-facing reason strings.
///
/// Mirrors the Kubernetes shape — `"timeout after 5s"`,
/// `"timeout after 250ms"` — so the renderer can pass the string
/// through unchanged.
fn format_duration(d: Duration) -> String {
    let total_ms = d.as_millis();
    if total_ms >= 1000 && total_ms.is_multiple_of(1000) {
        format!("{}s", d.as_secs())
    } else if total_ms >= 1000 {
        // Non-integer seconds — render the fractional form.
        let secs = d.as_secs_f64();
        format!("{secs:.1}s")
    } else {
        format!("{total_ms}ms")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn format_duration_renders_whole_seconds() {
        assert_eq!(format_duration(Duration::from_secs(5)), "5s");
        assert_eq!(format_duration(Duration::from_secs(1)), "1s");
        assert_eq!(format_duration(Duration::from_secs(30)), "30s");
    }

    #[test]
    fn format_duration_renders_sub_second_as_millis() {
        assert_eq!(format_duration(Duration::from_millis(250)), "250ms");
        assert_eq!(format_duration(Duration::from_millis(1)), "1ms");
        assert_eq!(format_duration(Duration::from_millis(999)), "999ms");
    }

    #[test]
    fn format_duration_renders_fractional_seconds() {
        assert_eq!(format_duration(Duration::from_millis(1500)), "1.5s");
    }

    #[test]
    fn connection_refused_maps_to_named_reason() {
        let err = io::Error::from(io::ErrorKind::ConnectionRefused);
        assert_eq!(connect_error_to_reason(&err, Duration::from_secs(5)), "connection refused");
    }

    #[test]
    fn kernel_timeout_maps_to_timeout_string() {
        let err = io::Error::from(io::ErrorKind::TimedOut);
        assert_eq!(connect_error_to_reason(&err, Duration::from_secs(5)), "timeout after 5s");
    }

    #[test]
    fn dns_lookup_failure_maps_to_dns_reason() {
        let err = io::Error::other("failed to lookup address: nodename nor servname provided");
        let rendered = connect_error_to_reason(&err, Duration::from_secs(5));
        assert!(rendered.starts_with("dns: "), "expected DNS-shaped reason, got: {rendered:?}");
    }

    /// Kill the match-guard mutant on line 100: an `ErrorKind::Other`
    /// whose message does NOT contain "failed to lookup address" must
    /// fall through to the catch-all arm and render as
    /// `"connect failed: ..."` — not as the DNS arm. When the match
    /// guard is replaced with `true` (cargo-mutants), every `Other`
    /// error routes to the DNS arm and this assertion flips red.
    #[test]
    fn other_io_error_without_dns_message_does_not_map_to_dns_reason() {
        let err = io::Error::other("some other failure");
        let rendered = connect_error_to_reason(&err, Duration::from_secs(5));
        assert!(
            !rendered.starts_with("dns: "),
            "expected non-DNS reason for generic Other error, got: {rendered:?}"
        );
        assert!(
            rendered.starts_with("connect failed: "),
            "expected catch-all `connect failed:` prefix, got: {rendered:?}"
        );
    }
}
