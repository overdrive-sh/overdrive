//! `setns`-on-a-dedicated-`std::thread` helper (ADR-0085 D4).
//!
//! In-netns netlink AND in-netns `/proc/sys` writes require entering the
//! target netns via `nix::sched::setns(fd, CLONE_NEWNET)`, and netns
//! membership is **per-thread**. [`in_netns`] runs the caller's closure on a
//! dedicated, throwaway thread that is `setns`'d into the target namespace and
//! **joined** on completion — it MUST NOT run on a pooled tokio worker /
//! `spawn_blocking` thread, because `setns` permanently mutates the calling
//! thread's netns and would poison a thread returned to the runtime for reuse.
//!
//! Scaffolded here in slice 01-01 (present with the ADR-pinned signature so
//! the crate compiles and 01-02 can build on it); the per-alloc netns path
//! that drives it — and the thread-lifecycle unit test — land in slice 01-02.

use overdrive_core::id::NetnsName;

use crate::error::NetlinkError;

/// Run `f` on a dedicated throwaway thread that is `setns`'d into `netns`,
/// and return its result.
///
/// The thread is joined before this returns and is **never** returned to a
/// pool (a `setns`'d thread is unreachable by the tokio worker pool,
/// ADR-0085 D4). The `async` `provision` path awaits the join via
/// `spawn_blocking` at the call site (the join itself, not the setns).
///
/// The closure typically opens an in-netns rtnetlink handle and/or writes
/// `/proc/sys/net/**` — both of which observe the thread's netns membership.
///
/// # Errors
///
/// [`NetlinkError::Setns`] when `/var/run/netns/<netns>` cannot be opened or
/// `setns` fails; otherwise the closure's own [`NetlinkError`].
pub fn in_netns<T, F>(netns: &NetnsName, f: F) -> Result<T, NetlinkError>
where
    F: FnOnce() -> Result<T, NetlinkError> + Send,
    T: Send,
{
    let netns_name = netns.as_str().to_owned();
    let netns_path = format!("/var/run/netns/{netns_name}");

    // `std::thread::scope` gives a dedicated thread that borrows `f` (only
    // `Send`, not `'static`) and is GUARANTEED joined before the scope exits —
    // exactly the "spawn-per-call, joined, never pooled" shape D4 requires.
    std::thread::scope(|scope| {
        scope
            .spawn(move || -> Result<T, NetlinkError> {
                let fd = std::fs::File::open(&netns_path)
                    .map_err(|source| NetlinkError::setns(netns_name.clone(), source))?;
                nix::sched::setns(fd, nix::sched::CloneFlags::CLONE_NEWNET).map_err(|errno| {
                    NetlinkError::setns(
                        netns_name.clone(),
                        std::io::Error::from_raw_os_error(errno as i32),
                    )
                })?;
                f()
            })
            .join()
            .unwrap_or_else(|_| {
                Err(NetlinkError::setns(
                    netns.as_str().to_owned(),
                    std::io::Error::other("in_netns worker thread panicked"),
                ))
            })
    })
}
