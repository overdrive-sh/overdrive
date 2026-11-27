//! `upstream_host` — the host adapters of the two upstream ports (ADR-0358).
//!
//! The resolver-file reader is real; the exchange body is still a `todo!`
//! scaffold.

#![allow(
    clippy::todo,
    clippy::unused_async,
    reason = "HostUpstreamExchange is still a scaffold until DELIVER (ADR-0356)"
)]

use std::io;
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use super::upstream::{
    DnsTransport, ExchangeError, UpstreamConfigError, UpstreamExchange, UpstreamResolvers,
};

/// Reads nameserver candidates from a resolver file on every call.
pub struct ResolvConfUpstreams {
    path: PathBuf,
}

impl ResolvConfUpstreams {
    /// A reader over `path` (tests; production reads `/etc/resolv.conf`).
    #[must_use]
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait::async_trait]
impl UpstreamResolvers for ResolvConfUpstreams {
    async fn candidates(&self) -> Result<Vec<IpAddr>, UpstreamConfigError> {
        let bytes = tokio::fs::read(&self.path).await.map_err(|source| match source.kind() {
            io::ErrorKind::NotFound => UpstreamConfigError::Absent,
            _ => UpstreamConfigError::Unreadable { source },
        })?;
        Ok(nameserver_addresses(&String::from_utf8_lossy(&bytes)))
    }
}

/// The plain IPv4/IPv6 addresses of the `nameserver` lines, in file order.
/// Comments, other keywords, scoped (`fe80::1%eth0`) and malformed tokens are
/// skipped.
fn nameserver_addresses(contents: &str) -> Vec<IpAddr> {
    contents
        .lines()
        .filter_map(|line| {
            let mut tokens = line.split_whitespace();
            (tokens.next() == Some("nameserver")).then_some(())?;
            tokens.next()?.parse::<IpAddr>().ok()
        })
        .collect()
}

/// Tokio UDP/TCP exchange, one fresh socket per exchange.
pub struct HostUpstreamExchange;

#[async_trait::async_trait]
impl UpstreamExchange for HostUpstreamExchange {
    async fn exchange(
        &self,
        server: IpAddr,
        transport: DnsTransport,
        query: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, ExchangeError> {
        let _ = (server, transport, query, timeout);
        todo!(
            "upstream exchange: one fresh socket per exchange, first matching reply within the timeout"
        )
    }
}
