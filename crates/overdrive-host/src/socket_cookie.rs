//! Safe `SO_COOKIE` reader for gateway-owned upstream sockets.
//!
//! SCAFFOLD: true. The syscall path is DELIVER's and fails with SCAFFOLD text.
//! Dataplane-internal helper: it is not re-exported to `overdrive-gateway` and
//! is not a boot-adapter field.

#![allow(
    clippy::todo,
    clippy::unused_self,
    clippy::missing_const_for_fn,
    reason = "public-ingress DISTILL scaffold: bodies are DELIVER's"
)]

use std::os::fd::BorrowedFd;

use overdrive_core::public_ingress::SocketCookie;
use thiserror::Error;

/// Stateless production socket-cookie reader.
#[derive(Debug, Default)]
pub struct SocketCookieReader;

impl SocketCookieReader {
    /// Construct the stateless reader.
    pub const fn new() -> Self {
        Self
    }

    /// Prove the host can read a nonzero socket cookie.
    pub fn probe(&self) -> Result<(), SocketCookieReadError> {
        todo!("SCAFFOLD: SocketCookieReader::probe")
    }

    /// Read the stable kernel cookie for one borrowed socket.
    pub fn read(&self, _socket: BorrowedFd<'_>) -> Result<SocketCookie, SocketCookieReadError> {
        todo!("SCAFFOLD: SocketCookieReader::read")
    }
}

/// Closed `SO_COOKIE` read failures.
#[derive(Debug, Error)]
pub enum SocketCookieReadError {
    #[error("SO_COOKIE is unsupported")]
    Unsupported,
    #[error("SO_COOKIE syscall failed: {source}")]
    Syscall { source: std::io::Error },
    #[error("SO_COOKIE returned zero")]
    Zero,
}
