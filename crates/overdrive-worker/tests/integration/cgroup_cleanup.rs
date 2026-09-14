//! Per-allocation cleanup guard for real `/sys/fs/cgroup` integration tests.
//!
//! Without this guard, a test that panics or is SIGKILL'd while an allocation
//! scope is live leaves the scope and any workload PIDs behind under
//! `/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-*.scope` until
//! something explicitly removes them. The next test creating a scope with the
//! same name then hits `EEXIST` on the mkdir.
//!
//! The worker crate keeps this helper beside the integration entrypoint rather
//! than under a driver-specific module because it is shared by the cgroup
//! manager and real cgroup-fs tests. It has no `ObservationStore` to enumerate
//! live allocations from: each caller passes the allocation ID for its one
//! owned scope.

use std::path::PathBuf;
use std::time::Duration;

use overdrive_core::id::AllocationId;

/// RAII guard that mass-kills, reaps, and removes one allocation scope when
/// dropped. Best effort: a missing scope is benign because the production
/// cleanup path may already have removed it.
pub struct AllocCleanup {
    pub cgroup_root: PathBuf,
    pub alloc: AllocationId,
}

impl AllocCleanup {
    /// Construct a guard for the given allocation scope.
    pub const fn register(cgroup_root: PathBuf, alloc: AllocationId) -> Self {
        Self { cgroup_root, alloc }
    }
}

impl Drop for AllocCleanup {
    fn drop(&mut self) {
        let scope = self
            .cgroup_root
            .join("overdrive.slice/workloads.slice")
            .join(format!("{}.scope", self.alloc));

        // Read PIDs before mass-killing so waitpid can reap each child. The
        // cgroup.kill write alone leaves child processes as zombies.
        let pids: Vec<libc::pid_t> = std::fs::read_to_string(scope.join("cgroup.procs"))
            .ok()
            .map(|s| s.lines().filter_map(|line| line.trim().parse::<i32>().ok()).collect())
            .unwrap_or_default();

        // ENOENT means the scope is already gone; the production stop path
        // has completed cleanup and there is nothing left for this guard.
        let _ = std::fs::write(scope.join("cgroup.kill"), "1\n");

        // Reap each PID without blocking forever on a process the test does
        // not own. The bounded retry also allows the kernel's SIGKILL to
        // reach the child before waitpid observes it.
        for pid in pids {
            for _ in 0..20 {
                let mut status: libc::c_int = 0;
                // SAFETY: `waitpid` receives a real pid and a valid status
                // pointer. Its return value is intentionally best-effort;
                // the bounded loop handles either completion or failure.
                let result = unsafe { libc::waitpid(pid, &raw mut status, libc::WNOHANG) };
                if result == pid || result == -1 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        let _ = std::fs::remove_dir(&scope);
    }
}
