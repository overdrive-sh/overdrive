//! Network-interface IPv4 resolution helper.
//!
//! Step 02-01 of `backend-discovery-bridge-service-reachability`. The
//! production boot path calls [`resolve_iface_ipv4`] once at startup
//! against the operator-supplied `[dataplane] client_iface` value to
//! obtain the host IPv4 address the `BackendDiscoveryBridge` writes
//! into every `service_backends` observation row's `endpoint.host`
//! field (architecture.md § 5.2).
//!
//! Per `.claude/rules/development.md` § Errors → "Distinct failure
//! modes get distinct error variants" the two failure modes surface
//! distinctly:
//!
//! - `io::ErrorKind::NotFound` — the interface exists in `getifaddrs`
//!   output but carries no IPv4 address (e.g. an IPv6-only veth). The
//!   composition root routes this through
//!   [`crate::error::DataplaneBootError::IfaceAddrResolution`] whose
//!   `Display` form suggests `ip -4 addr show <iface>`.
//! - `io::ErrorKind::Other` — `getifaddrs` itself returned a system
//!   error (typically `EACCES` from an unprivileged process or
//!   resource starvation). Same routing; the operator message names
//!   the iface and the underlying cause via `{source}`.
//!
//! The helper is `#[cfg(any(linux_android, bsd, solarish))]` via the
//! `nix::ifaddrs` gating; macOS dev hosts (where the inner loop runs
//! via `cargo xtask lima run --`) resolve `lo` (or another configured
//! iface) inside the Lima VM where it is reachable.

use std::net::Ipv4Addr;

/// Resolve a single IPv4 address bound to `iface`.
///
/// Walks `getifaddrs(3)` and returns the first IPv4 address whose
/// `interface_name` matches `iface`. Returns
/// `Err(io::ErrorKind::NotFound)` when the interface has no IPv4
/// address (the interface may not exist OR may be IPv6-only — both
/// shapes surface as `NotFound` because the operator's remediation is
/// the same: `ip -4 addr show <iface>` to inspect). Returns
/// `Err(io::ErrorKind::Other)` when `getifaddrs` itself fails.
///
/// # Errors
///
/// - [`std::io::ErrorKind::NotFound`] — no IPv4 address resolved for
///   the named interface (missing iface OR iface present without
///   IPv4 binding).
/// - [`std::io::ErrorKind::Other`] — `getifaddrs(3)` returned a
///   system error (the `Display` form names the iface and the
///   underlying `nix::Errno`).
pub fn resolve_iface_ipv4(iface: &str) -> std::io::Result<Ipv4Addr> {
    let addrs = nix::ifaddrs::getifaddrs().map_err(|errno| {
        std::io::Error::other(format!("getifaddrs failed for iface {iface}: {errno}"))
    })?;

    for ifaddr in addrs {
        if ifaddr.interface_name != iface {
            continue;
        }
        if let Some(addr) = ifaddr.address
            && let Some(sin) = addr.as_sockaddr_in()
        {
            // `sin.ip()` returns the IPv4 in host byte order.
            return Ok(sin.ip());
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!(
            "no IPv4 address found for interface {iface}; \
             inspect with `ip -4 addr show {iface}`"
        ),
    ))
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "test code: expect is the canonical assertion pattern")]
mod tests {
    use std::io::ErrorKind;
    use std::net::Ipv4Addr;

    use super::resolve_iface_ipv4;

    /// Loopback always exists on every supported test host. On Linux
    /// it is `lo`; on macOS dev hosts (the outer-loop entry point)
    /// it is `lo0`. Both bind `127.0.0.1` per RFC 1122.
    #[test]
    fn resolve_iface_ipv4_loopback() {
        // Try both common loopback names — Linux `lo` and macOS `lo0`
        // — so the unit test runs cleanly on both the Lima VM (Linux,
        // `lo`) and the macOS dev host (no Lima wrapper for in-crate
        // unit tests).
        let resolved = resolve_iface_ipv4("lo")
            .or_else(|_| resolve_iface_ipv4("lo0"))
            .expect("loopback interface must resolve on every supported test host");
        assert_eq!(resolved, Ipv4Addr::LOCALHOST);
    }

    #[test]
    fn resolve_iface_ipv4_nonexistent_iface_returns_not_found() {
        let err =
            resolve_iface_ipv4("bogus-iface-foo").expect_err("bogus iface name must not resolve");
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }
}
