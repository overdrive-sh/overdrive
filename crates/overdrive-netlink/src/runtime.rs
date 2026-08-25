//! The sync → async netlink bridge (ADR-0085 D4/D5).
//!
//! The per-alloc / per-intercept provisioning surface is **sync** — called
//! from the async action-shim on a tokio worker (`veth_provisioner`), or from a
//! blocking `std::net` accept loop (`mtls_intercept`) — so it cannot `block_on`
//! rtnetlink ops on the calling thread: that panics with "runtime within a
//! runtime". Every netlink op therefore runs on a **dedicated throwaway
//! `std::thread`** that builds its own current-thread runtime the rtnetlink
//! `Client` connection is spawned on:
//!
//! - HOST-netns ops via [`block_on_host_netlink`] (a fresh host-netns thread);
//! - IN-NETNS ops via [`crate::in_netns`] (its own setns'd dedicated thread)
//!   with [`block_on_netlink`] inside the closure.
//!
//! This is the single auditable home for the bridge, mirroring
//! [`crate::in_netns`] — both consumers share one implementation instead of
//! duplicating it verbatim.

use crate::error::NetlinkError;

/// Build a fresh current-thread tokio runtime and drive `fut` to completion.
///
/// Used INSIDE an [`crate::in_netns`] closure (a `setns`'d dedicated thread
/// with no ambient runtime) and by [`block_on_host_netlink`] (a fresh
/// host-netns thread) — both need a runtime the rtnetlink `Client` connection
/// is spawned on.
///
/// # Errors
///
/// [`NetlinkError::Connect`] when the current-thread runtime cannot be built;
/// otherwise the future's own [`NetlinkError`].
pub fn block_on_netlink<T>(
    fut: impl std::future::Future<Output = Result<T, NetlinkError>>,
) -> Result<T, NetlinkError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(NetlinkError::connect)?
        .block_on(fut)
}

/// Run an async netlink/netns closure on a dedicated throwaway `std::thread` in
/// the HOST netns, blocking the caller until it completes.
///
/// The thread inherits the caller's (host) netns and is never a pooled tokio
/// worker, so its current-thread runtime can `block_on` without the "runtime
/// within a runtime" panic. Joined before returning, so nothing outlives the
/// call.
///
/// # Errors
///
/// [`NetlinkError::Connect`] when the worker thread panics; otherwise the
/// closure's own [`NetlinkError`].
pub fn block_on_host_netlink<T, F, Fut>(f: F) -> Result<T, NetlinkError>
where
    F: FnOnce() -> Fut + Send,
    Fut: std::future::Future<Output = Result<T, NetlinkError>>,
    T: Send,
{
    std::thread::scope(|scope| {
        scope.spawn(|| block_on_netlink(f())).join().unwrap_or_else(|_| {
            Err(NetlinkError::connect(std::io::Error::other("host netlink worker thread panicked")))
        })
    })
}
