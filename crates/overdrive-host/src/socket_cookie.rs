//! Safe `SO_COOKIE` reader for gateway-owned upstream sockets.
//!
//! Dataplane-internal helper: it is not re-exported to `overdrive-gateway` and
//! is not a boot-adapter field. `read` goes through rustix's safe
//! `sockopt::socket_cookie` wrapper (`getsockopt(SOL_SOCKET, SO_COOKIE)`,
//! `net/core/sock.c:2109-2113 @ v7.0`), so the crate's `deny(unsafe_code)`
//! stays intact.

#![allow(
    clippy::todo,
    clippy::unused_self,
    clippy::missing_const_for_fn,
    reason = "`probe` is the boot sentinel step's (public-ingress-gateway)"
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
    ///
    /// # Errors
    ///
    /// [`SocketCookieReadError::Unsupported`] when the kernel has no
    /// `SO_COOKIE` (`ENOPROTOOPT`), [`SocketCookieReadError::Syscall`] for any
    /// other failure, [`SocketCookieReadError::Zero`] when the kernel reports 0.
    pub fn read(&self, socket: BorrowedFd<'_>) -> Result<SocketCookie, SocketCookieReadError> {
        let raw = rustix::net::sockopt::socket_cookie(socket).map_err(|errno| {
            if errno == rustix::io::Errno::NOPROTOOPT {
                SocketCookieReadError::Unsupported
            } else {
                SocketCookieReadError::Syscall { source: errno.into() }
            }
        })?;
        SocketCookie::new(raw).map_err(|_| SocketCookieReadError::Zero)
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
