//! Force OS thread creation to fail with `EAGAIN`.
//!
//! Moves the current process into a cgroup v2 scratch leaf capped at its
//! current task count, so the next `clone`/`pthread_create` is refused.
//!
//! GH #295 E23 (user decision 2 of 2026-09-30): "OUR CODE MUST NEVER PANIC. IT
//! MUST ALWAYS BE RECOVERABLE AND/OR SELF HEALING." Under the release profile's
//! `panic = "abort"` a panic on any owner, kill, or supervisor path would end
//! every workload's network owner with no fail-stop. The no-panic contract is
//! exercised by refusing the one resource those paths need to create a worker
//! thread — an OS thread — and asserting each subject returns its typed failure
//! (`NetlinkError::Connect`, a per-TAP `unconfirmed` entry, or a kill write's
//! `io::Error`) rather than aborting.
//!
//! Real kernel only; requires root + cgroup v2 (the Lima dev VM). The kernel's
//! `pids` controller returns `EAGAIN` from `cgroup_can_fork`
//! (`kernel/cgroup/pids.c`) once `pids.current` reaches `pids.max`. `nextest`
//! runs each test in its own process, so capping this process affects only the
//! one test.

#![cfg(target_os = "linux")]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;

const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// RAII guard that makes every new OS thread this process creates fail with
/// `EAGAIN` while it is held.
///
/// On drop it moves the process back to its original cgroup and removes the
/// scratch leaf, so a panicking subject (the pre-fix behaviour this lane is RED
/// against) still restores the process on unwind.
#[derive(Debug)]
pub struct ThreadCreationRefused {
    origin: PathBuf,
    scratch: PathBuf,
}

impl Drop for ThreadCreationRefused {
    fn drop(&mut self) {
        // Best-effort restore: move the process out of the capped scratch first
        // (so it can create threads again and the scratch empties), then remove
        // the scratch leaf.
        let _ = fs::write(self.origin.join("cgroup.procs"), process_line());
        let _ = fs::remove_dir(&self.scratch);
    }
}

fn process_line() -> String {
    format!("{}\n", process::id())
}

/// This process's current cgroup v2 path (the `0::<path>` line of
/// `/proc/self/cgroup`).
fn current_cgroup() -> io::Result<PathBuf> {
    let content = fs::read_to_string("/proc/self/cgroup")?;
    let relative = content
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .ok_or_else(|| io::Error::other("no cgroup v2 `0::` line in /proc/self/cgroup"))?
        .trim();
    Ok(Path::new(CGROUP_ROOT).join(relative.trim_start_matches('/')))
}

/// Number of tasks (threads) in this process right now.
fn current_task_count() -> io::Result<usize> {
    Ok(fs::read_dir("/proc/self/task")?.count())
}

/// Cap this process's thread count so the next thread creation is refused.
///
/// Moves the process into a fresh scratch leaf under the cgroup root, capped at
/// its current task count, so the next `clone`/`pthread_create` returns
/// `EAGAIN` for the guard's lifetime.
///
/// The scratch is a child of the root cgroup (which may hold processes and
/// delegate controllers — the cgroup v2 root exemption), so the move does not
/// trip the "no internal processes" rule that a shared `nextest` cgroup would.
///
/// # Errors
///
/// Any cgroup filesystem step the kernel refuses — most importantly a missing
/// `pids.max`, which means the substrate did not delegate the `pids` controller
/// to the root's children. The caller treats that as "cannot run here" and
/// skips, rather than asserting on a cap that was never installed.
pub fn refuse_thread_creation() -> io::Result<ThreadCreationRefused> {
    let origin = current_cgroup()?;
    let scratch = Path::new(CGROUP_ROOT).join(format!("ovd-e23-{}", process::id()));

    // Delegate the pids controller from the root to its children. The root may
    // do this with member processes present (root exemption); writing `+pids`
    // when it is already enabled is a no-op.
    let _ = fs::write(Path::new(CGROUP_ROOT).join("cgroup.subtree_control"), "+pids\n");

    fs::create_dir_all(&scratch)?;
    // Move the process in while `pids.max` is still the default `max`, then cap
    // it at the task count now that every task lives in the scratch.
    fs::write(scratch.join("cgroup.procs"), process_line())?;
    let guard = ThreadCreationRefused { origin, scratch: scratch.clone() };
    let tasks = current_task_count()?;
    fs::write(scratch.join("pids.max"), format!("{tasks}\n")).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "capping {} pids.max at {tasks} failed ({error}); the substrate must delegate the \
                 `pids` controller to the cgroup root's children",
                scratch.display()
            ),
        )
    })?;
    Ok(guard)
}
