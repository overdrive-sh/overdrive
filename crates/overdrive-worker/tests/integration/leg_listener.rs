//! The `LegListener` bridge (GH #295, S-ND295-70) — test support, not
//! production API (`docs/feature/netns-density-295/distill/test-scenarios.md`
//! § *Intercept listener and stop-error test support*).
//!
//! [`MtlsIntercept::bind_transparent`] returns a `std::net::TcpListener` until
//! the DELIVER step that carries DISTILL gap B-7 (no later than 05-01), and a
//! port-owned `Arc<dyn InterceptListener>` from then on (feature delta
//! § *Driven port — intercept listener*). Test code in this integration binary
//! that reads a bound leg's address, or accepts one connection on it, goes
//! through [`LegListener`], so every body compiles and keeps its oracle on both
//! sides of that step. The B-7 step deletes the `std::net::TcpListener`
//! implementation below, which then has no caller, and changes no body.
//!
//! Consumers: the port equivalence harness (`mtls_intercept_equivalence.rs`),
//! `MetalSharedIntercept` (`outbound_enforce_substrate_splice.rs`), the
//! host-listener bodies of `mtls_intercept_install.rs`, and the
//! original-destination steps of `egress_tproxy_capture.rs` and
//! `name_resolve_enforce_consistency.rs`.
//!
//! # Accept failures
//!
//! [`LegListener::accept_leg`] returns an [`io::Error`] whose inner error is
//! the [`InterceptAcceptError`] the accept produced, so a body can tell the
//! terminal `Accept` outcome from the connection-scoped `OriginalDestination`
//! outcome on both sides of the B-7 step ([`accept_failure_of`]). Before that
//! step the `TcpListener` implementation maps the production accept helper's
//! `InterceptError::Accept` and `InterceptError::OrigDst` onto those two
//! outcomes, which is the partition the worker's accept loop applies today.
//!
//! [`MtlsIntercept::bind_transparent`]: overdrive_worker::mtls_intercept_port::MtlsIntercept::bind_transparent

use std::io;
use std::net::{SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::os::fd::OwnedFd;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use overdrive_worker::mtls_intercept::{InterceptError, accept_outbound_and_recover_orig_dst};
use overdrive_worker::mtls_intercept_port::{InterceptAcceptError, InterceptListener};

/// One accepted leg: the owned stream, the peer the accept reported, and the
/// accepted connection's local address (on the host, the destination the peer
/// originally dialled).
pub type AcceptedLeg = (OwnedFd, SocketAddrV4, SocketAddrV4);

/// A bound intercept leg, read and accepted the same way on both sides of the
/// B-7 step.
pub trait LegListener {
    /// The leg's bound IPv4 address. A non-IPv4 bind is an
    /// [`io::ErrorKind::InvalidData`] error.
    fn bound_v4(&self) -> io::Result<SocketAddrV4>;

    /// Block until exactly one connection is accepted, returning
    /// `(stream, peer, local)`. A failure carries its [`InterceptAcceptError`]
    /// as the inner error ([`accept_failure_of`]).
    fn accept_leg(&self) -> io::Result<AcceptedLeg>;
}

/// Today's listener: the accept goes through the production outbound helper
/// `accept_outbound_and_recover_orig_dst`, and the peer is read from the
/// accepted socket. Deleted by the B-7 step.
impl LegListener for TcpListener {
    fn bound_v4(&self) -> io::Result<SocketAddrV4> {
        ipv4(self.local_addr()?)
    }

    fn accept_leg(&self) -> io::Result<AcceptedLeg> {
        let (stream, local) =
            accept_outbound_and_recover_orig_dst(self).map_err(helper_accept_failure)?;
        let stream = TcpStream::from(stream);
        let peer = ipv4(stream.peer_addr()?)?;
        Ok((OwnedFd::from(stream), peer, local))
    }
}

/// The port-owned listener: the accept is driven on a current-thread Tokio
/// runtime this call builds.
#[allow(
    dead_code,
    reason = "RED scaffold: no caller until the DELIVER step that carries B-7 (no later than \
              05-01) changes bind_transparent's return type; that step deletes the TcpListener \
              impl above and every bridge caller uses this one"
)]
impl LegListener for Arc<dyn InterceptListener> {
    fn bound_v4(&self) -> io::Result<SocketAddrV4> {
        InterceptListener::local_addr(self.as_ref())
    }

    fn accept_leg(&self) -> io::Result<AcceptedLeg> {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        let accepted =
            runtime.block_on(InterceptListener::accept(self.as_ref())).map_err(accept_failure)?;
        Ok((accepted.stream, accepted.peer, accepted.local))
    }
}

/// The [`InterceptAcceptError`] a failed [`LegListener::accept_leg`] carries,
/// or `None` when the failure is not an accept outcome (a runtime that could
/// not be built, an unreadable peer address).
#[must_use]
pub fn accept_failure_of(error: &io::Error) -> Option<&InterceptAcceptError> {
    error.get_ref().and_then(|inner| inner.downcast_ref::<InterceptAcceptError>())
}

/// Start one [`LegListener::accept_leg`] on its own thread and return the
/// channel its outcome arrives on. The thread holds a clone of `listener`
/// until its accept completes.
pub fn spawn_accept_leg<L>(listener: &Arc<L>) -> io::Result<mpsc::Receiver<io::Result<AcceptedLeg>>>
where
    L: LegListener + Send + Sync + 'static,
{
    let (outcome_tx, outcome_rx) = mpsc::channel();
    let listener = Arc::clone(listener);
    std::thread::Builder::new().name("leg-accept".to_owned()).spawn(move || {
        let outcome = listener.accept_leg();
        // A waiter that timed out has dropped its receiver; the outcome then
        // has no reader, which is not a setup failure.
        if let Err(unread) = outcome_tx.send(outcome) {
            drop(unread);
        }
    })?;
    Ok(outcome_rx)
}

/// Accept one connection on `listener`, waiting at most `within`. An accept
/// that has not completed by then is an [`io::ErrorKind::TimedOut`] error; its
/// thread keeps a clone of `listener` until the accept completes, so a body
/// treats that outcome as a failure and does not reuse the listener.
pub fn accept_leg_within<L>(listener: &Arc<L>, within: Duration) -> io::Result<AcceptedLeg>
where
    L: LegListener + Send + Sync + 'static,
{
    match spawn_accept_leg(listener)?.recv_timeout(within) {
        Ok(outcome) => outcome,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("no connection was accepted on the leg within {within:?}"),
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err(io::Error::other("the leg accept thread ended without an outcome"))
        }
    }
}

fn ipv4(address: SocketAddr) -> io::Result<SocketAddrV4> {
    match address {
        SocketAddr::V4(address) => Ok(address),
        SocketAddr::V6(address) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("intercept leg reported an IPv6 address {address}"),
        )),
    }
}

fn accept_failure(error: InterceptAcceptError) -> io::Error {
    let kind = match &error {
        InterceptAcceptError::Accept { source }
        | InterceptAcceptError::OriginalDestination { source } => source.kind(),
    };
    io::Error::new(kind, error)
}

fn helper_accept_failure(error: InterceptError) -> io::Error {
    match error {
        InterceptError::Accept { source, .. } => {
            accept_failure(InterceptAcceptError::Accept { source })
        }
        InterceptError::OrigDst { source } => {
            accept_failure(InterceptAcceptError::OriginalDestination { source })
        }
        other => io::Error::other(other),
    }
}
