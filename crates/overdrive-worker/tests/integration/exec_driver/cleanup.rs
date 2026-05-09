//! Per-alloc cleanup guard for `exec_driver` integration tests that
//! run against real `/sys/fs/cgroup` (Phase 02 of
//! `fix-cgroup-subtree-control-delegation`).
//!
//! Without this guard, a test that panics or is SIGKILL'd between
//! `Driver::start` and `Driver::stop` leaves the alloc scope (and any
//! workload PIDs inside it) behind under
//! `/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-*.scope`
//! until something explicitly removes them. The next test creating a
//! scope with the same name then hits `EEXIST` on the mkdir.
//!
//! Mirrors the `ScopeCleanup` shape from
//! `crates/overdrive-control-plane/tests/integration/cgroup_isolation/
//! alloc_scope_has_writable_cpu_weight_and_memory_max.rs` (the worker
//! crate cannot reuse it cross-crate). Differs from
//! `crates/overdrive-control-plane/tests/integration/job_lifecycle/
//! cleanup.rs::AllocCleanup` in that we do NOT have an
//! `ObservationStore` to enumerate live allocs from — the test passes
//! the `AllocationId` directly because each test owns exactly one
//! scope.
//!
//! At drop time:
//! 1. Read each scope's `cgroup.procs` for the live PIDs.
//! 2. Write `1\n` to `cgroup.kill` to deliver SIGKILL atomically.
//! 3. `waitpid(WNOHANG)` each PID a handful of times to reap zombies
//!    so `nextest`'s leak detector finds no surviving child of the
//!    test process.
//! 4. `rmdir` the scope.

use std::path::PathBuf;
use std::time::Duration;

use overdrive_core::id::AllocationId;

/// RAII guard that mass-kills + reaps + rmdirs a single alloc scope
/// when dropped. Best-effort — every step is `let _ = ...` so a
/// missing scope (because production `Driver::stop` already removed
/// it on the happy path) is benign.
pub struct AllocCleanup {
    pub cgroup_root: PathBuf,
    pub alloc: AllocationId,
}

impl AllocCleanup {
    /// Construct a guard for the given alloc scope. Idiomatic shape:
    ///
    /// ```ignore
    /// let alloc = AllocationId::new("alloc-foo").unwrap();
    /// let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());
    /// // ... rest of test body ...
    /// ```
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

        // Read PIDs out of cgroup.procs BEFORE mass-killing so we can
        // `waitpid` each one and reap the zombie. nextest's leak
        // detector flags any unreaped child of the test process;
        // cgroup.kill alone leaves them as zombies.
        let pids: Vec<libc::pid_t> = std::fs::read_to_string(scope.join("cgroup.procs"))
            .ok()
            .map(|s| s.lines().filter_map(|line| line.trim().parse::<i32>().ok()).collect())
            .unwrap_or_default();

        // Mass-kill every process in the cgroup. Best-effort — ENOENT
        // means the scope is already gone (the production stop path
        // already cleaned up).
        let _ = std::fs::write(scope.join("cgroup.kill"), "1\n");

        // Reap every PID we collected. SIGKILL has been queued by
        // cgroup.kill above; the kernel will surface SIGCHLD on each
        // child shortly. waitpid with WNOHANG and a small retry loop
        // avoids any race against signal delivery without blocking
        // forever on a PID we don't actually own.
        for pid in pids {
            for _ in 0..20 {
                let mut status: libc::c_int = 0;
                // SAFETY: `waitpid` is a thin syscall wrapper. We
                // pass a real pid_t and a valid status pointer;
                // ignoring the return is sound because the loop bails
                // on the next read or after 20×10ms.
                let r = unsafe { libc::waitpid(pid, &raw mut status, libc::WNOHANG) };
                if r == pid || r == -1 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        let _ = std::fs::remove_dir(&scope);
    }
}
