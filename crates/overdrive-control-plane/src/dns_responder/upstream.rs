//! `upstream` — the two driven ports of the forwarder and the pure upstream
//! selection (GPD-1, GPD-3; ADR-0356, ADR-0358).
//!
//! DISTILL scaffold: shapes are the pinned contract; `select` is `todo!`.

#![allow(
    clippy::todo,
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_value,
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    dead_code,
    reason = "DISTILL scaffold: bodies are todo until DELIVER (ADR-0356, ADR-0358)"
)]

use std::io;
use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use thiserror::Error;

/// Where the host's nameserver candidates come from.
#[async_trait::async_trait]
pub trait UpstreamResolvers: Send + Sync {
    /// Current nameserver candidates of the host, in file order.
    async fn candidates(&self) -> Result<Vec<IpAddr>, UpstreamConfigError>;
}

/// One request-reply exchange with an upstream resolver.
#[async_trait::async_trait]
pub trait UpstreamExchange: Send + Sync {
    /// Send `query` to `server:53` over `transport` on a fresh socket and
    /// return the first matching reply, within `timeout`.
    async fn exchange(
        &self,
        server: IpAddr,
        transport: DnsTransport,
        query: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, ExchangeError>;
}

/// The transport of one exchange.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DnsTransport {
    /// UDP.
    Udp,
    /// TCP, RFC 1035 two-byte framing.
    Tcp,
}

/// Why the host's candidates could not be read.
#[derive(Debug, Error)]
pub enum UpstreamConfigError {
    /// The source does not exist.
    #[error("resolver file is absent")]
    Absent,
    /// The source exists but could not be read.
    #[error("resolver file is unreadable: {source}")]
    Unreadable {
        /// The read failure.
        source: io::Error,
    },
}

/// Why one exchange failed.
#[derive(Debug, Error)]
pub enum ExchangeError {
    /// No matching reply within the timeout.
    #[error("upstream did not answer in time")]
    Timeout,
    /// A refused connect or an ICMP error.
    #[error("upstream unreachable: {source}")]
    Unreachable {
        /// The OS error.
        source: io::Error,
    },
    /// Any other socket failure.
    #[error("upstream exchange failed: {source}")]
    Io {
        /// The OS error.
        source: io::Error,
    },
    /// A UDP datagram filled the receive buffer.
    #[error("upstream reply too large")]
    ReplyTooLarge,
}

/// The candidates left after dropping the gateway.
#[derive(Clone, Debug)]
pub struct UpstreamSet {
    servers: Vec<IpAddr>,
}

/// Every candidate was the gateway, or there were none.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
#[error("no usable upstream resolver")]
pub struct NoUsableUpstream;

impl UpstreamSet {
    /// Pure selection: drop only the gateway (IPv4 and IPv4-mapped IPv6 form),
    /// keep order and count.
    pub fn select(candidates: Vec<IpAddr>, gateway: Ipv4Addr) -> Result<Self, NoUsableUpstream> {
        let servers: Vec<IpAddr> = candidates
            .into_iter()
            .filter(|candidate| !is_gateway(*candidate, gateway))
            .collect();
        if servers.is_empty() {
            return Err(NoUsableUpstream);
        }
        Ok(Self { servers })
    }
}

/// The gateway in its IPv4 form or its IPv4-mapped IPv6 form.
fn is_gateway(candidate: IpAddr, gateway: Ipv4Addr) -> bool {
    match candidate {
        IpAddr::V4(v4) => v4 == gateway,
        IpAddr::V6(v6) => v6.to_ipv4_mapped() == Some(gateway),
    }
}
