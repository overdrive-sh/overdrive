//! `upstream_host` — the host adapters of the two upstream ports (ADR-0358).
//!
//! DISTILL scaffold: constructors are real; port bodies are `todo!`.

#![allow(
    clippy::todo,
    clippy::missing_const_for_fn,
    clippy::unused_async,
    clippy::needless_pass_by_value,
    clippy::doc_markdown,
    reason = "DISTILL scaffold: bodies are todo until DELIVER (ADR-0358)"
)]

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
        let _ = &self.path;
        todo!("resolver file: re-read the file on every call and list its nameserver addresses")
    }
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
