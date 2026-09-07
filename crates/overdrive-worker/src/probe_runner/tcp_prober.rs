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

use std::io;
use std::net::SocketAddr;
use std::os::fd::AsRawFd;
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
use overdrive_core::traits::prober::{ProbeFailure, ProbeOutcome, TcpProber};
use tokio::net::{TcpSocket, TcpStream};

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

        match tokio::time::timeout(timeout, connect_marked(host, port)).await {
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

/// Connect through the existing Tokio host-resolution path, applying the
/// worker's trusted-dial mark before each non-loopback candidate sends its
/// SYN. The mark selects the already-installed OUTPUT exemption; it does not
/// rewrite the target or alter candidate order.
async fn connect_marked(host: &str, port: u16) -> io::Result<TcpStream> {
    let addresses = tokio::net::lookup_host((host, port)).await?;
    let mut last_error = None;

    for address in addresses {
        match socket_for_probe_target(address)?.connect(address).await {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "tcp probe host resolved to no addresses")
    }))
}

/// Create a TCP probe socket and stamp the existing trusted-dial mark before
/// a non-loopback SYN can reach the worker's OUTPUT TPROXY rule.
fn socket_for_probe_target(address: SocketAddr) -> io::Result<TcpSocket> {
    let socket = match address {
        SocketAddr::V4(_) => TcpSocket::new_v4()?,
        SocketAddr::V6(_) => TcpSocket::new_v6()?,
    };

    if !address.ip().is_loopback() {
        set_agent_dial_mark(&socket)?;
    }

    Ok(socket)
}

/// Stamp the existing agent-dial `SO_MARK` on an unconnected TCP socket.
fn set_agent_dial_mark(socket: &TcpSocket) -> io::Result<()> {
    let mark = MTLS_LEG_S_DIAL_MARK;
    let mark_len = libc::socklen_t::try_from(std::mem::size_of_val(&mark)).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "socket mark length exceeds socklen_t")
    })?;
    // SAFETY: `TcpSocket` owns this live fd, and Linux `SO_MARK` reads exactly
    // one `u32`. This runs before `TcpSocket::connect`, so the SYN carries the
    // pre-existing worker recursion-exemption mark.
    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_MARK,
            std::ptr::from_ref(&mark).cast(),
            mark_len,
        )
    };
    if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
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

    fn socket_mark(socket: &TcpSocket) -> io::Result<u32> {
        let mut mark = 0_u32;
        let mut mark_len =
            libc::socklen_t::try_from(std::mem::size_of_val(&mark)).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "socket mark length exceeds socklen_t")
            })?;
        // SAFETY: `socket` owns this live fd, and `getsockopt` writes at most
        // `mark_len` bytes into the correctly-sized `mark` buffer.
        let result = unsafe {
            libc::getsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_MARK,
                std::ptr::from_mut(&mut mark).cast(),
                std::ptr::from_mut(&mut mark_len),
            )
        };
        if result == 0 { Ok(mark) } else { Err(io::Error::last_os_error()) }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn non_loopback_tcp_probe_socket_has_the_agent_mark_before_connect() -> io::Result<()> {
        let socket = socket_for_probe_target(SocketAddr::from(([192, 0, 2, 42], 8080)))?;

        assert_eq!(socket_mark(&socket)?, MTLS_LEG_S_DIAL_MARK);
        Ok(())
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn loopback_tcp_probe_socket_remains_unmarked_before_connect() -> io::Result<()> {
        let socket = socket_for_probe_target(SocketAddr::from(([127, 0, 0, 1], 8080)))?;

        assert_eq!(socket_mark(&socket)?, 0);
        Ok(())
    }

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
