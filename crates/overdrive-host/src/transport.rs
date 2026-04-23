//! Host [`Transport`] binding.
//!
//! Phase 1 ships the type so `overdrive-core`'s trait bounds can be
//! discharged by the wiring layer. The network methods still return
//! `Unsupported` — Phase 2 wires them to `tokio::net::*`.

use std::io;
use std::net::SocketAddr;

use async_trait::async_trait;
use bytes::Bytes;
use overdrive_core::traits::transport::{Connection, Transport, TransportError};

/// Production TCP transport. Placeholder until Phase 2 wires it to
/// `tokio::net::*`.
#[derive(Debug, Default)]
pub struct TcpTransport {
    _private: (),
}

#[async_trait]
impl Transport for TcpTransport {
    async fn connect(&self, addr: SocketAddr) -> Result<Box<dyn Connection>, TransportError> {
        Err(TransportError::Connect {
            addr,
            source: io::Error::new(
                io::ErrorKind::Unsupported,
                "TcpTransport::connect — Phase 2 wires this to tokio::net",
            ),
        })
    }

    async fn send_datagram(
        &self,
        addr: SocketAddr,
        _payload: Bytes,
    ) -> Result<usize, TransportError> {
        Err(TransportError::Connect {
            addr,
            source: io::Error::new(
                io::ErrorKind::Unsupported,
                "TcpTransport::send_datagram — Phase 2 wires this to tokio::net",
            ),
        })
    }
}
