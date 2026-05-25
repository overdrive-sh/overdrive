//! C-cgroup-kill — writing `1\n` to `cgroup.kill` mass-kills every PID
//! in the scope, and the kernel reaps them within seconds.
//!
//! Tier 3, real-io. Requires Lima sudo (writes to
//! `/sys/fs/cgroup/overdrive.slice/workloads.slice/...`).
//!
//! Exercises ADR-0054 § D3 row 1 (cgroup.kill atomic mass-kill) — a
//! semantic `SimCgroupFs` CANNOT model. The Sim adapter stores the
//! `b"1\n"` payload at the target path; it does NOT terminate any
//! process because no real PID exists in the in-memory store. This
//! scenario is the structural defense that the production code path
//! lands on `RealCgroupFs` and that the trait surface preserves the
//! kernel-side effect.
//!
//! Scenario reference: `docs/feature/cgroup-fs-port/distill/test-scenarios.md`
//! § C-cgroup-kill.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::id::AllocationId;
use overdrive_core::traits::CgroupFs;
use overdrive_host::RealCgroupFs;
use overdrive_worker::cgroup_manager::CgroupManager;
use serial_test::serial;

use super::super::exec_driver::cleanup::AllocCleanup;

#[tokio::test]
#[serial(cgroup)]
async fn cgroup_kill_terminates_every_pid_within_two_seconds() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    CgroupManager::new(cgroup_root.to_path_buf(), fs.clone())
        .create_workloads_slice_with_controllers()
        .await
        .expect("workloads.slice bootstrap succeeds");

    let alloc = AllocationId::new("alloc-killC-0").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());

    let scope_dir = cgroup_root.join(format!("overdrive.slice/workloads.slice/{alloc}.scope"));
    fs.create_dir(&scope_dir).await.expect("create alloc scope");

    // Spawn `/bin/sleep 3600` and move it into the scope via
    // `cgroup.procs`. Mirrors the production wiring in
    // `ExecDriver::start` (post-spawn PID move) but without the rest
    // of the driver machinery — this test exercises ONLY the
    // cgroup.kill semantic, not the full driver surface.
    let child =
        tokio::process::Command::new("/bin/sleep").arg("3600").spawn().expect("spawn /bin/sleep");
    // Linux PIDs are non-negative integers bounded by the kernel's
    // `kernel.pid_max` (≤ 2^22), well within `i32::MAX`. The cast is
    // safe by domain.
    #[allow(clippy::cast_possible_wrap)]
    let pid = child.id().expect("child PID populated") as libc::pid_t;

    fs.write(&scope_dir.join("cgroup.procs"), format!("{pid}\n").as_bytes())
        .await
        .expect("place PID in cgroup.procs");

    // Mass-kill.
    fs.write(&scope_dir.join("cgroup.kill"), b"1\n").await.expect("cgroup.kill write");

    // Poll waitpid(WNOHANG) up to 2 seconds wall-clock. The kernel
    // delivers SIGKILL to every PID in the cgroup atomically; the
    // child should reap promptly.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut reaped = false;
    while Instant::now() < deadline {
        let mut status: libc::c_int = 0;
        // SAFETY: thin syscall wrapper. `pid` is a real pid_t and
        // `&raw mut status` is a valid pointer.
        let r = unsafe { libc::waitpid(pid, &raw mut status, libc::WNOHANG) };
        if r == pid {
            reaped = true;
            break;
        }
        if r == -1 {
            // Already reaped by SIGCHLD handler (rare under tokio,
            // but the kernel signal could race ahead of the poll).
            reaped = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        reaped,
        "PID {pid} not reaped within 2s after cgroup.kill — \
         kernel-side semantic broken",
    );
}
