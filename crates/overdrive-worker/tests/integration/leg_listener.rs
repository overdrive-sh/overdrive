//! The `LegListener` bridge (GH #295, S-ND295-70) — test support, not
//! production API (`docs/feature/netns-density-295/distill/test-scenarios.md`
//! § *Intercept listener and stop-error test support*).
//!
//! [`MtlsIntercept::bind_transparent`] returns the port-owned
//! `Arc<dyn InterceptListener>`. Test code in this integration binary that
//! reads a bound leg's address or accepts one connection uses [`LegListener`].
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
//! outcome ([`accept_failure_of`]).
//!
//! [`MtlsIntercept::bind_transparent`]: overdrive_worker::mtls_intercept_port::MtlsIntercept::bind_transparent

use std::io;
use std::net::SocketAddrV4;
use std::os::fd::OwnedFd;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use overdrive_worker::mtls_intercept_port::{InterceptAcceptError, InterceptListener};

/// One accepted leg: the owned stream, the peer the accept reported, and the
/// accepted connection's local address (on the host, the destination the peer
/// originally dialled).
pub type AcceptedLeg = (OwnedFd, SocketAddrV4, SocketAddrV4);

/// A bound intercept leg read and accepted through its port.
pub trait LegListener {
    /// The leg's bound IPv4 address. A non-IPv4 bind is an
    /// [`io::ErrorKind::InvalidData`] error.
    fn bound_v4(&self) -> io::Result<SocketAddrV4>;

    /// Block until exactly one connection is accepted, returning
    /// `(stream, peer, local)`. A failure carries its [`InterceptAcceptError`]
    /// as the inner error ([`accept_failure_of`]).
    fn accept_leg(&self) -> io::Result<AcceptedLeg>;
}

/// The port-owned listener: the accept is driven on a current-thread Tokio
/// runtime this call builds.
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

fn accept_failure(error: InterceptAcceptError) -> io::Error {
    let kind = match &error {
        InterceptAcceptError::Accept { source }
        | InterceptAcceptError::OriginalDestination { source } => source.kind(),
    };
    io::Error::new(kind, error)
}
