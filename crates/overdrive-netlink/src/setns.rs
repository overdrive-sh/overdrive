//! `setns`-on-a-dedicated-`std::thread` helper (ADR-0085 D4).
//!
//! In-netns netlink AND in-netns `/proc/sys` reads require entering the target
//! netns via `nix::sched::setns(fd, CLONE_NEWNET)`, and netns membership is
//! **per-thread**. [`in_netns`] runs the caller's closure on a dedicated,
//! throwaway thread that is `setns`'d into the target namespace and **joined**
//! on completion — it MUST NOT run on a pooled tokio worker / `spawn_blocking`
//! thread, because `setns` permanently mutates the calling thread's netns and
//! would poison a thread returned to the runtime for reuse.
//!
//! The per-alloc `provision_workload_netns` path that drives it is a **sync**
//! fn (called from the async action-shim on a tokio worker), so it invokes
//! [`in_netns`] directly and blocks on the join — the dedicated thread has no
//! ambient tokio runtime, so the closure is free to build its own
//! current-thread runtime and drive the in-netns rtnetlink ops without the
//! "runtime within a runtime" panic a `block_on` on the calling worker would
//! hit.
//!
//! The "spawn a fresh thread per call, run the closure there, join before
//! returning, never pool the thread" guarantee is factored into
//! [`run_on_dedicated_thread`] so the D4 thread-lifecycle property is unit-
//! testable in the default lane without a real netns (entering a netns needs
//! `CAP_NET_ADMIN` + an existing `/var/run/netns/<name>`). [`in_netns`] is that
//! primitive composed with the `setns` entry.

use overdrive_core::id::NetnsName;

use crate::error::NetlinkError;

/// Run `work` to completion on a dedicated, throwaway `std::thread` and return
/// its result (or the panic payload if `work` unwound).
///
/// The thread is spawned **fresh per call** (a distinct
/// [`std::thread::ThreadId`], guaranteed never reused for the process lifetime)
/// and **joined** before this returns, via [`std::thread::scope`] — it is
/// **never** a pooled tokio worker. This is the exact structural guarantee
/// [`in_netns`] (ADR-0085 D4) needs: `setns` permanently mutates the running
/// thread's netns, so the entered thread must die on completion, never be
/// returned to a runtime for reuse.
///
/// Exposed at crate visibility so the D4 thread-lifecycle unit test can assert
/// the spawn-per-call / joined / not-the-caller properties without needing a
/// real netns (which requires `CAP_NET_ADMIN`).
pub(crate) fn run_on_dedicated_thread<T, F>(work: F) -> std::thread::Result<T>
where
    F: FnOnce() -> T + Send,
    T: Send,
{
    std::thread::scope(|scope| scope.spawn(work).join())
}

/// Enter `netns` on the CURRENT thread via `setns(CLONE_NEWNET)`. Called only
/// from inside [`run_on_dedicated_thread`] (a throwaway thread), never on a
/// pooled thread, because it permanently mutates the thread's netns membership.
fn enter_netns(netns_name: &str) -> Result<(), NetlinkError> {
    let netns_path = format!("/var/run/netns/{netns_name}");
    let fd = std::fs::File::open(&netns_path)
        .map_err(|source| NetlinkError::setns(netns_name.to_owned(), source))?;
    nix::sched::setns(fd, nix::sched::CloneFlags::CLONE_NEWNET).map_err(|errno| {
        NetlinkError::setns(netns_name.to_owned(), std::io::Error::from_raw_os_error(errno as i32))
    })
}

/// Run `f` on a dedicated throwaway thread that is `setns`'d into `netns`,
/// and return its result.
///
/// The thread is joined before this returns and is **never** returned to a
/// pool (a `setns`'d thread is unreachable by the tokio worker pool,
/// ADR-0085 D4). The sync `provision_workload_netns` path calls this directly
/// and blocks on the join; the closure — running on a thread with no ambient
/// tokio runtime — is free to build a current-thread runtime and drive the
/// in-netns rtnetlink ops (and/or read `/proc/sys/net/**`, which observes the
/// thread's netns membership).
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
    run_on_dedicated_thread(move || {
        enter_netns(&netns_name)?;
        f()
    })
    .unwrap_or_else(|_| {
        Err(NetlinkError::setns(
            netns.as_str().to_owned(),
            std::io::Error::other("in_netns worker thread panicked"),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::run_on_dedicated_thread;
    use std::thread::ThreadId;

    /// ADR-0085 D4 thread-lifecycle lock (single-example — no pure property to
    /// generalise): the dedicated-thread runner
    ///
    ///   1. runs the closure on a thread that is NOT the caller's (never the
    ///      pooled tokio worker that invoked `provision_workload_netns`),
    ///   2. JOINS it (the closure's return value propagates back), and
    ///   3. spawns a FRESH thread PER call (a distinct `ThreadId`, which the
    ///      stdlib guarantees is never reused for the process lifetime — so two
    ///      calls observing two different ids proves neither reuse nor pooling).
    ///
    /// This targets `run_on_dedicated_thread` — the primitive `in_netns`
    /// composes with the `setns` entry — so the property is provable in the
    /// default lane without `CAP_NET_ADMIN` / a real `/var/run/netns/<name>`.
    #[test]
    fn dedicated_thread_runs_off_caller_joins_and_is_fresh_per_call() {
        let caller: ThreadId = std::thread::current().id();

        // (1)+(2): the closure runs on a non-caller thread, and its result is
        // joined back to us.
        let first: ThreadId =
            run_on_dedicated_thread(|| std::thread::current().id()).expect("worker joined");
        assert_ne!(first, caller, "closure must run on a dedicated thread, not the caller/pool");

        // (3): a second call runs on ANOTHER fresh thread — distinct ThreadIds
        // prove spawn-per-call (a pooled/reused thread would repeat an id).
        let second: ThreadId =
            run_on_dedicated_thread(|| std::thread::current().id()).expect("worker joined");
        assert_ne!(second, caller, "second call must also run off the caller thread");
        assert_ne!(first, second, "each call must spawn a FRESH thread (never pooled/reused)");
    }

    /// The runner JOINS: a value computed on the dedicated thread is returned
    /// to the caller intact (not dropped on the floor).
    #[test]
    fn dedicated_thread_returns_the_closure_value() {
        let out: u64 = run_on_dedicated_thread(|| 40_u64 + 2).expect("worker joined");
        assert_eq!(out, 42, "the joined closure's return value must propagate back");
    }
}
