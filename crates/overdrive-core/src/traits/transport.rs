//! [`Transport`] — the sole source of networking for Overdrive logic.
//!
//! All TCP / QUIC traffic initiated by control-plane, node agent, gateway,
//! or reconciler code goes through this trait. Direct `tokio::net::*` usage
//! is forbidden outside wiring crates so that DST can partition, delay, or
//! drop connections deterministically.

use std::net::SocketAddr;

use async_trait::async_trait;
use bytes::Bytes;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("connect to {addr} failed: {source}")]
    Connect {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    #[error("bind to {addr} failed: {source}")]
    Bind {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    #[error("peer closed connection")]
    Closed,
    #[error("transport I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// Bidirectional byte stream over the injected transport.
pub trait Connection: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static {}

impl<T> Connection for T where T: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static {}

#[async_trait]
pub trait Transport: Send + Sync + 'static {
    /// Open an outbound connection to `addr`.
    async fn connect(&self, addr: SocketAddr) -> Result<Box<dyn Connection>, TransportError>;

    /// Send a single datagram. Returns the number of bytes sent.
    async fn send_datagram(
        &self,
        addr: SocketAddr,
        payload: Bytes,
    ) -> Result<usize, TransportError>;
}
