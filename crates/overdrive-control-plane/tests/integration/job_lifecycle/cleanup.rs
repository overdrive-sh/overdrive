//! Test cleanup helper shared across `job_lifecycle` walking-skeleton
//! tests that spawn real `/bin/sleep` workloads via `ExecDriver`.
//!
//! Without this guard, the workloads survive past the `#[tokio::test]`
//! boundary and `nextest` flags the test as `LEAK`. The guard runs at
//! drop time and goes straight to the kernel:
//!
//! 1. Read each scope's `cgroup.procs` for the live PIDs.
//! 2. Write `1\n` to `cgroup.kill` to deliver SIGKILL atomically.
//! 3. `waitpid(WNOHANG)` each PID a handful of times to reap zombies
//!    so `nextest`'s leak detector finds no surviving child of the
//!    test process.
//! 4. `rmdir` the scope.
//!
//! We deliberately do NOT call `Driver::stop` from the cleanup path:
//! `tokio::process::Child::wait()` is registered with the runtime
//! that spawned the child. The `#[tokio::test]` runtime is tearing
//! down at drop time, so awaiting on a fresh `tokio::runtime::Runtime`
//! would hang indefinitely waiting for a SIGCHLD signal that the
//! spawning reactor is the only one wired to receive.

use std::sync::Arc;

use overdrive_core::traits::observation_store::ObservationStore;

/// Test cleanup guard — see module docs.
pub struct AllocCleanup {
    pub obs: Arc<dyn ObservationStore>,
    pub cgroup_root: std::path::PathBuf,
}

impl Drop for AllocCleanup {
    fn drop(&mut self) {
        // Use a fresh runtime ONLY to read the obs store (in-process,
        // no Child handles involved — safe across runtimes). All
        // workload termination happens via direct cgroupfs writes.
        let obs = self.obs.clone();
        let rows = std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Runtime::new() else { return Vec::new() };
            rt.block_on(async move { obs.alloc_status_rows().await.unwrap_or_default() })
        })
        .join()
        .unwrap_or_default();

        for row in rows {
            let scope = self
                .cgroup_root
                .join("overdrive.slice/workloads.slice")
                .join(format!("{}.scope", row.alloc_id));

            // Read PIDs out of cgroup.procs BEFORE mass-killing so we
            // can `waitpid` each one and reap the zombie. nextest's
            // leak detector flags any unreaped child of the test
            // process; cgroup.kill alone leaves them as zombies.
            let pids: Vec<libc::pid_t> = std::fs::read_to_string(scope.join("cgroup.procs"))
                .ok()
                .map(|s| s.lines().filter_map(|line| line.trim().parse::<i32>().ok()).collect())
                .unwrap_or_default();

            // Mass-kill every process in the cgroup. Best-effort —
            // ENOENT means the scope is already gone.
            let _ = std::fs::write(scope.join("cgroup.kill"), "1\n");

            // Reap every PID we collected. SIGKILL has been queued by
            // cgroup.kill above; the kernel will surface SIGCHLD on
            // each child shortly. waitpid with WNOHANG and a small
            // retry loop avoids any race against signal delivery
            // without blocking forever on a PID we don't actually own
            // (e.g. unrelated processes that happened to share the
            // cgroup before delegation).
            for pid in pids {
                for _ in 0..20 {
                    let mut status: libc::c_int = 0;
                    // SAFETY: `waitpid` is a thin syscall wrapper.
                    // Passing a real pid_t and a valid status pointer
                    // is sound; we ignore the return because the loop
                    // bails on the next read or after 20×10ms.
                    let r = unsafe { libc::waitpid(pid, &raw mut status, libc::WNOHANG) };
                    if r == pid || r == -1 {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }

            let _ = std::fs::remove_dir(&scope);
        }
    }
}
